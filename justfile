name := 'cosmic-camera-controls'
appid := 'com.github.ctsdownloads.CameraControls'

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))

bin-src := 'target' / 'release' / name
bin-dst := base-dir / 'bin' / name

desktop := appid + '.desktop'
desktop-src := 'resources' / 'app.desktop'
desktop-dst := clean(rootdir / prefix) / 'share' / 'applications' / desktop

icon-src := 'resources' / 'icons' / 'scalable' / 'apps' / appid + '.svg'
icon-dst := clean(rootdir / prefix) / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg'

metainfo := appid + '.metainfo.xml'
metainfo-src := 'resources' / 'metainfo.xml'
metainfo-dst := clean(rootdir / prefix) / 'share' / 'metainfo' / metainfo

# Default recipe — build release binary
default: build-release

# Compile in release mode
build-release:
    cargo build --release

# Compile in debug mode
build-debug:
    cargo build

# Run release build directly
run: build-release
    ./{{bin-src}}

# Run clippy linter
check:
    cargo clippy --all-features -- -W clippy::pedantic

# Run tests
test:
    cargo test

# Install binary, desktop file, icon, and metainfo
install:
    install -Dm0755 {{bin-src}} {{bin-dst}}
    install -Dm0644 {{desktop-src}} {{desktop-dst}}
    install -Dm0644 {{icon-src}} {{icon-dst}}
    install -Dm0644 {{metainfo-src}} {{metainfo-dst}}

# Remove installed files
uninstall:
    rm -f {{bin-dst}}
    rm -f {{desktop-dst}}
    rm -f {{icon-dst}}
    rm -f {{metainfo-dst}}

# Clean build artifacts
clean:
    cargo clean
