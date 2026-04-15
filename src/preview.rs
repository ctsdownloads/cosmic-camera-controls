use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use v4l::buffer::Type;
use v4l::device::Device;
use v4l::io::mmap::Stream;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;
use v4l::FourCC;

/// A decoded RGBA frame ready for display
#[derive(Clone)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Handle to a running preview — drop to stop capture
pub struct PreviewHandle {
    pub rx: mpsc::Receiver<Frame>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PreviewHandle {
    pub fn start(dev_path: PathBuf) -> Result<Self, String> {
        let (frame_tx, frame_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();

        let handle = thread::Builder::new()
            .name("camera-preview".into())
            .spawn(move || {
                if let Err(e) = capture_loop(&dev_path, frame_tx, stop_rx) {
                    log::error!("Preview capture error: {}", e);
                }
            })
            .map_err(|e| format!("Failed to spawn preview thread: {}", e))?;

        Ok(PreviewHandle {
            rx: frame_rx,
            stop: Some(stop_tx),
            thread: Some(handle),
        })
    }

    pub fn stop(&mut self) {
        self.stop.take(); // Signal thread by dropping sender
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PreviewHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn should_stop(stop: &mpsc::Receiver<()>) -> bool {
    match stop.try_recv() {
        Ok(()) | Err(mpsc::TryRecvError::Disconnected) => true,
        Err(mpsc::TryRecvError::Empty) => false,
    }
}

fn capture_loop(
    dev_path: &Path,
    tx: mpsc::Sender<Frame>,
    stop: mpsc::Receiver<()>,
) -> Result<(), String> {
    // Retry device open — previous holder may still be releasing
    let mut last_err = String::from("unknown");

    for attempt in 0..15 {
        if should_stop(&stop) {
            return Ok(());
        }

        let mut dev = match Device::with_path(dev_path) {
            Ok(d) => d,
            Err(e) => {
                last_err = format!("open: {}", e);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };

        let fmt = match dev.format() {
            Ok(f) => f,
            Err(e) => {
                last_err = format!("format: {}", e);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };

        let width = fmt.width;
        let height = fmt.height;
        let fourcc = fmt.fourcc;

        let mut stream = match Stream::with_buffers(&mut dev, Type::VideoCapture, 4) {
            Ok(s) => s,
            Err(e) => {
                last_err = format!("stream: {}", e);
                drop(dev);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };

        log::info!(
            "Preview: {}x{} fourcc={} from {} (attempt {})",
            width, height, fourcc, dev_path.display(), attempt
        );

        // Capture loop — stream and dev live together in this scope
        let target_interval = Duration::from_millis(66);

        loop {
            if should_stop(&stop) {
                return Ok(());
            }

            let frame_start = Instant::now();

            let (buf, _meta) = match stream.next() {
                Ok(frame) => frame,
                Err(e) => {
                    log::warn!("Frame capture error: {}", e);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            let rgba = match decode_frame(buf, &fourcc, width, height) {
                Some(data) => data,
                None => continue,
            };

            let frame = Frame { rgba, width, height };

            if tx.send(frame).is_err() {
                return Ok(()); // Receiver dropped
            }

            let elapsed = frame_start.elapsed();
            if elapsed < target_interval {
                thread::sleep(target_interval - elapsed);
            }
        }
    }

    Err(format!("Failed after retries: {}", last_err))
}

/// Decode a raw frame buffer to RGBA based on the pixel format
fn decode_frame(buf: &[u8], fourcc: &FourCC, width: u32, height: u32) -> Option<Vec<u8>> {
    let fourcc_bytes: [u8; 4] = fourcc.repr;

    match &fourcc_bytes {
        b"MJPG" => decode_mjpeg(buf),
        b"YUYV" => Some(decode_yuyv(buf, width, height)),
        b"RGB3" | b"RGB4" => {
            if fourcc_bytes == *b"RGB4" {
                Some(buf.to_vec())
            } else {
                Some(rgb_to_rgba(buf))
            }
        }
        _ => {
            decode_mjpeg(buf).or_else(|| {
                log::warn!("Unsupported pixel format: {}", fourcc);
                None
            })
        }
    }
}

fn decode_mjpeg(buf: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(buf).ok()?;
    Some(img.to_rgba8().into_raw())
}

fn decode_yuyv(buf: &[u8], width: u32, height: u32) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgba = vec![255u8; pixel_count * 4];

    for i in 0..(pixel_count / 2) {
        let base = i * 4;
        if base + 3 >= buf.len() {
            break;
        }

        let y0 = buf[base] as f32;
        let u = buf[base + 1] as f32 - 128.0;
        let y1 = buf[base + 2] as f32;
        let v = buf[base + 3] as f32 - 128.0;

        let out0 = i * 2 * 4;
        let out1 = (i * 2 + 1) * 4;

        rgba[out0] = clamp_u8(y0 + 1.402 * v);
        rgba[out0 + 1] = clamp_u8(y0 - 0.344 * u - 0.714 * v);
        rgba[out0 + 2] = clamp_u8(y0 + 1.772 * u);
        rgba[out0 + 3] = 255;

        rgba[out1] = clamp_u8(y1 + 1.402 * v);
        rgba[out1 + 1] = clamp_u8(y1 - 0.344 * u - 0.714 * v);
        rgba[out1 + 2] = clamp_u8(y1 + 1.772 * u);
        rgba[out1 + 3] = 255;
    }

    rgba
}

fn rgb_to_rgba(buf: &[u8]) -> Vec<u8> {
    let pixel_count = buf.len() / 3;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for chunk in buf.chunks_exact(3) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
    rgba
}

fn clamp_u8(v: f32) -> u8 {
    v.max(0.0).min(255.0) as u8
}
