use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use v4l::capability::Flags as CapFlags;
use v4l::device::Device;
use v4l::frameinterval::FrameIntervalEnum;
use v4l::framesize::FrameSizeEnum;
use v4l::video::Capture;
use v4l::FourCC;

// ── Raw V4L2 ioctl definitions ──────────────────────────────────────────
// Bypass the v4l crate for control enumeration because v4l 0.14
// panics internally in Description::from() on some cameras.

// ioctl request codes (x86_64 / aarch64)
const VIDIOC_QUERYCTRL: libc::c_ulong = 0xC044_5624; // _IOWR('V', 36, v4l2_queryctrl)
const VIDIOC_QUERYMENU: libc::c_ulong = 0xC02C_5625; // _IOWR('V', 37, v4l2_querymenu)
const VIDIOC_G_CTRL: libc::c_ulong = 0xC008_561B;    // _IOWR('V', 27, v4l2_control)
const VIDIOC_S_CTRL: libc::c_ulong = 0xC008_561C;    // _IOWR('V', 28, v4l2_control)

// Control types
const V4L2_CTRL_TYPE_INTEGER: u32 = 1;
const V4L2_CTRL_TYPE_BOOLEAN: u32 = 2;
const V4L2_CTRL_TYPE_MENU: u32 = 3;
const V4L2_CTRL_TYPE_BUTTON: u32 = 4;
const V4L2_CTRL_TYPE_INTEGER64: u32 = 5;
const V4L2_CTRL_TYPE_INTEGER_MENU: u32 = 9;

// Control flags
const V4L2_CTRL_FLAG_DISABLED: u32 = 0x0001;
const V4L2_CTRL_FLAG_READ_ONLY: u32 = 0x0004;
const V4L2_CTRL_FLAG_NEXT_CTRL: u32 = 0x8000_0000;

#[repr(C)]
#[derive(Clone)]
struct V4l2Queryctrl {
    id: u32,
    type_: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}

impl V4l2Queryctrl {
    fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    fn name_str(&self) -> String {
        CStr::from_bytes_until_nul(&self.name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|_| String::from("Unknown"))
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct V4l2Querymenu {
    id: u32,
    index: u32,
    name: [u8; 32],
    reserved: u32,
}

impl V4l2Querymenu {
    fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }

    fn name_str(&self) -> String {
        CStr::from_bytes_until_nul(&self.name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|_| String::from("Unknown"))
    }
}

#[repr(C)]
struct V4l2Control {
    id: u32,
    value: i32,
}

// ── Public types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraId {
    pub vendor_id: String,
    pub product_id: String,
    pub serial: Option<String>,
}

impl CameraId {
    pub fn profile_key(&self) -> String {
        match &self.serial {
            Some(s) if !s.is_empty() => format!("{}:{}:{}", self.vendor_id, self.product_id, s),
            _ => format!("{}:{}", self.vendor_id, self.product_id),
        }
    }
}

impl std::fmt::Display for CameraId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.profile_key())
    }
}

#[derive(Debug, Clone)]
pub struct CameraInfo {
    pub id: CameraId,
    pub name: String,
    pub dev_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CameraControl {
    pub id: u32,
    pub name: String,
    pub ctrl_type: ControlKind,
    pub default: i64,
}

#[derive(Debug, Clone)]
pub enum ControlKind {
    Integer { min: i64, max: i64, step: i64 },
    Boolean,
    Menu { items: Vec<(i64, String)> },
    Button,
}

#[derive(Debug, Clone)]
pub struct FormatOption {
    pub fourcc: FourCC,
    pub width: u32,
    pub height: u32,
    pub framerates: Vec<(u32, u32)>,
}

// ── Device enumeration ──────────────────────────────────────────────────

pub fn enumerate_cameras() -> Vec<CameraInfo> {
    let mut cameras = Vec::new();

    for index in 0..16 {
        let path = PathBuf::from(format!("/dev/video{}", index));
        if !path.exists() {
            continue;
        }

        let Ok(dev) = Device::with_path(&path) else {
            continue;
        };

        let Ok(caps) = dev.query_caps() else {
            continue;
        };

        if !caps.capabilities.contains(CapFlags::VIDEO_CAPTURE) {
            continue;
        }

        let name = caps.card.clone();
        let id = read_usb_identity(&path);

        cameras.push(CameraInfo {
            id,
            name,
            dev_path: path,
        });
    }

    cameras
}

fn read_usb_identity(dev_path: &Path) -> CameraId {
    let fallback = CameraId {
        vendor_id: "0000".into(),
        product_id: "0000".into(),
        serial: None,
    };

    let Ok(mut enumerator) = udev::Enumerator::new() else {
        return fallback;
    };

    let dev_name = dev_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let _ = enumerator.match_subsystem("video4linux");

    for device in enumerator.scan_devices().into_iter().flatten() {
        let sysname = device.sysname().to_string_lossy();
        if sysname != dev_name {
            continue;
        }

        if let Some(usb_dev) = device
            .parent_with_subsystem_devtype("usb", "usb_device")
            .ok()
            .flatten()
        {
            let vendor = usb_dev
                .attribute_value("idVendor")
                .map(|v| v.to_string_lossy().to_string())
                .unwrap_or_else(|| "0000".into());
            let product = usb_dev
                .attribute_value("idProduct")
                .map(|v| v.to_string_lossy().to_string())
                .unwrap_or_else(|| "0000".into());
            let serial = usb_dev
                .attribute_value("serial")
                .map(|v| v.to_string_lossy().to_string());

            return CameraId {
                vendor_id: vendor,
                product_id: product,
                serial,
            };
        }
    }

    fallback
}

// ── Raw V4L2 control enumeration ────────────────────────────────────────

/// Open a V4L2 device for raw ioctl access
fn open_device_fd(dev_path: &Path) -> Option<std::fs::File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev_path)
        .ok()
}

/// Enumerate controls via raw ioctl — bypasses v4l crate's broken query_controls()
pub fn enumerate_controls(dev_path: &Path) -> Vec<CameraControl> {
    let Some(file) = open_device_fd(dev_path) else {
        log::error!("Failed to open {} for control enumeration", dev_path.display());
        return Vec::new();
    };
    let fd = file.as_raw_fd();
    let mut controls = Vec::new();
    let mut qctrl = V4l2Queryctrl::zeroed();

    // Start enumeration with NEXT_CTRL flag to walk all controls
    qctrl.id = V4L2_CTRL_FLAG_NEXT_CTRL;

    loop {
        let ret = unsafe { libc::ioctl(fd, VIDIOC_QUERYCTRL, &mut qctrl as *mut _) };
        if ret < 0 {
            break; // No more controls
        }

        // Skip disabled and read-only
        if (qctrl.flags & V4L2_CTRL_FLAG_DISABLED) != 0
            || (qctrl.flags & V4L2_CTRL_FLAG_READ_ONLY) != 0
        {
            qctrl.id |= V4L2_CTRL_FLAG_NEXT_CTRL;
            continue;
        }

        let ctrl_type = match qctrl.type_ {
            V4L2_CTRL_TYPE_INTEGER | V4L2_CTRL_TYPE_INTEGER64 => ControlKind::Integer {
                min: qctrl.minimum as i64,
                max: qctrl.maximum as i64,
                step: qctrl.step as i64,
            },
            V4L2_CTRL_TYPE_BOOLEAN => ControlKind::Boolean,
            V4L2_CTRL_TYPE_MENU | V4L2_CTRL_TYPE_INTEGER_MENU => {
                let items = query_menu_items(fd, &qctrl);
                ControlKind::Menu { items }
            }
            V4L2_CTRL_TYPE_BUTTON => ControlKind::Button,
            _ => {
                // Unknown control type — skip
                qctrl.id |= V4L2_CTRL_FLAG_NEXT_CTRL;
                continue;
            }
        };

        controls.push(CameraControl {
            id: qctrl.id,
            name: qctrl.name_str(),
            ctrl_type,
            default: qctrl.default_value as i64,
        });

        // Advance to next control
        qctrl.id |= V4L2_CTRL_FLAG_NEXT_CTRL;
    }

    log::info!("Enumerated {} controls via raw ioctl", controls.len());
    controls
}

/// Query menu items for a menu-type control
fn query_menu_items(fd: i32, qctrl: &V4l2Queryctrl) -> Vec<(i64, String)> {
    let mut items = Vec::new();

    for index in (qctrl.minimum as u32)..=(qctrl.maximum as u32) {
        let mut qmenu = V4l2Querymenu::zeroed();
        qmenu.id = qctrl.id;
        qmenu.index = index;

        let ret = unsafe { libc::ioctl(fd, VIDIOC_QUERYMENU, &mut qmenu as *mut _) };
        if ret < 0 {
            continue; // This index not valid — sparse menus are allowed
        }

        let label = if qctrl.type_ == V4L2_CTRL_TYPE_INTEGER_MENU {
            // For integer menus, the value is in the name field as an i64
            // but we'll just show the index
            format!("{}", index)
        } else {
            qmenu.name_str()
        };

        items.push((index as i64, label));
    }

    items
}

// ── Raw V4L2 control get/set ────────────────────────────────────────────

pub fn get_control_value(dev_path: &Path, id: u32) -> Option<i64> {
    let file = open_device_fd(dev_path)?;
    let fd = file.as_raw_fd();
    let mut ctrl = V4l2Control { id, value: 0 };

    let ret = unsafe { libc::ioctl(fd, VIDIOC_G_CTRL, &mut ctrl as *mut _) };
    if ret < 0 {
        None
    } else {
        Some(ctrl.value as i64)
    }
}

/// Error from setting a V4L2 control
pub struct ControlError {
    pub errno: i32,
    pub message: String,
}

impl ControlError {
    pub fn is_permission_denied(&self) -> bool {
        self.errno == libc::EACCES || self.errno == libc::EPERM
    }
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub fn set_control_value(dev_path: &Path, id: u32, value: i64) -> Result<(), ControlError> {
    let file = open_device_fd(dev_path)
        .ok_or_else(|| ControlError {
            errno: 0,
            message: format!("Failed to open device for control {}", id),
        })?;
    let fd = file.as_raw_fd();
    let mut ctrl = V4l2Control {
        id,
        value: value as i32,
    };

    let ret = unsafe { libc::ioctl(fd, VIDIOC_S_CTRL, &mut ctrl as *mut _) };
    if ret < 0 {
        let err = std::io::Error::last_os_error();
        let raw = err.raw_os_error().unwrap_or(0);
        Err(ControlError {
            errno: raw,
            message: format!("Failed to set control {}: {}", id, err),
        })
    } else {
        Ok(())
    }
}

pub fn snapshot_controls(dev_path: &Path, controls: &[CameraControl]) -> HashMap<u32, i64> {
    let mut values = HashMap::new();
    for ctrl in controls {
        if let Some(val) = get_control_value(dev_path, ctrl.id) {
            values.insert(ctrl.id, val);
        }
    }
    values
}

pub fn apply_controls(
    dev_path: &Path,
    values: &HashMap<u32, i64>,
    controls: &[CameraControl],
) -> Vec<String> {
    // Apply in dependency order: boolean/menu controls first (auto mode toggles),
    // then integer controls (manual values). This ensures auto modes are set
    // before their dependent manual controls are attempted.

    // Build ordered list: booleans and menus first, then integers
    let mut auto_ids: Vec<u32> = Vec::new();
    let mut manual_ids: Vec<u32> = Vec::new();

    for ctrl in controls {
        if values.contains_key(&ctrl.id) {
            match ctrl.ctrl_type {
                ControlKind::Boolean | ControlKind::Menu { .. } => auto_ids.push(ctrl.id),
                _ => manual_ids.push(ctrl.id),
            }
        }
    }

    // Also include IDs not in the controls list (from saved profile)
    for &id in values.keys() {
        if !auto_ids.contains(&id) && !manual_ids.contains(&id) {
            manual_ids.push(id);
        }
    }

    // First: apply auto mode toggles
    for &id in &auto_ids {
        if let Some(&value) = values.get(&id) {
            if let Err(e) = set_control_value(dev_path, id, value) {
                log::warn!("{}", e);
            }
        }
    }

    // Then: apply manual controls
    let mut errors = Vec::new();
    for &id in &manual_ids {
        if let Some(&value) = values.get(&id) {
            if let Err(e) = set_control_value(dev_path, id, value) {
                if e.is_permission_denied() {
                    // EACCES/EPERM means the control is auto-managed — not an error,
                    // the hardware is controlling this value because auto mode is active
                    let name = controls
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    log::info!("Control '{}' is auto-managed, skipping", name);
                } else {
                    errors.push(e.message);
                }
            }
        }
    }

    errors
}

// ── Format enumeration (still uses v4l crate — this works fine) ─────────

pub fn enumerate_formats(dev: &Device) -> Vec<FormatOption> {
    let mut options = Vec::new();

    let Ok(formats) = dev.enum_formats() else {
        return options;
    };

    for fmt_desc in formats {
        let fourcc = fmt_desc.fourcc;

        let Ok(sizes) = dev.enum_framesizes(fourcc) else {
            continue;
        };

        for size in sizes {
            match size.size {
                FrameSizeEnum::Discrete(discrete) => {
                    let framerates = enum_framerates(dev, fourcc, discrete.width, discrete.height);
                    options.push(FormatOption {
                        fourcc,
                        width: discrete.width,
                        height: discrete.height,
                        framerates,
                    });
                }
                FrameSizeEnum::Stepwise(stepwise) => {
                    let common = [
                        (640, 480),
                        (1280, 720),
                        (1920, 1080),
                        (2560, 1440),
                        (3840, 2160),
                    ];
                    for (w, h) in common {
                        if w >= stepwise.min_width
                            && w <= stepwise.max_width
                            && h >= stepwise.min_height
                            && h <= stepwise.max_height
                        {
                            let framerates = enum_framerates(dev, fourcc, w, h);
                            options.push(FormatOption {
                                fourcc,
                                width: w,
                                height: h,
                                framerates,
                            });
                        }
                    }
                }
            }
        }
    }

    options
}

fn enum_framerates(dev: &Device, fourcc: FourCC, width: u32, height: u32) -> Vec<(u32, u32)> {
    let Ok(intervals) = dev.enum_frameintervals(fourcc, width, height) else {
        return vec![];
    };

    intervals
        .into_iter()
        .filter_map(|fi| match fi.interval {
            FrameIntervalEnum::Discrete(d) => Some((d.numerator, d.denominator)),
            _ => None,
        })
        .collect()
}

/// Apply a format (resolution + pixel format) to the device
pub fn set_format(
    dev_path: &Path,
    fourcc: FourCC,
    width: u32,
    height: u32,
) -> Result<(u32, u32), String> {
    let dev =
        Device::with_path(dev_path).map_err(|e| format!("Failed to open device: {}", e))?;

    let mut fmt = dev
        .format()
        .map_err(|e| format!("Failed to read format: {}", e))?;

    fmt.fourcc = fourcc;
    fmt.width = width;
    fmt.height = height;

    let actual = dev
        .set_format(&fmt)
        .map_err(|e| format!("Failed to set format: {}", e))?;

    log::info!(
        "Format set: requested {}x{} {}, got {}x{} {}",
        width, height, fourcc,
        actual.width, actual.height, actual.fourcc
    );

    Ok((actual.width, actual.height))
}
