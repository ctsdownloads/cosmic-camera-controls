name := 'cosmic-camera-controls'
appid := 'io.github.ctsdownloads.CosmicCameraControls'

prefix := '/usr'
bin-src := 'target/release/' + name
bin-dst := prefix + '/bin/' + name
desktop-src := 'res/' + appid + '.desktop'
desktop-dst := prefix + '/share/applications/' + appid + '.desktop'
icon-src := 'res/' + appid + '.svg'
icon-dst := prefix + '/share/icons/hicolor/scalable/apps/' + appid + '.svg'

default: build-release

# Run the app from a terminal for debugging
run:
    cargo run

# Build optimized release binary
build-release:
    cargo build --release

# Build and install (requires sudo for /usr paths)
install: build-release
    sudo install -Dm0755 {{bin-src}} {{bin-dst}}
    sudo install -Dm0644 {{desktop-src}} {{desktop-dst}}
    sudo install -Dm0644 {{icon-src}} {{icon-dst}}

# Remove installed files
uninstall:
    sudo rm -f {{bin-dst}}
    sudo rm -f {{desktop-dst}}
    sudo rm -f {{icon-dst}}
