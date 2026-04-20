# justfile for github.com/ctsdownloads/cosmic-camera-controls
# Three-file install: binary + .desktop + icon

name := 'cosmic-camera-controls'
appid := 'io.github.ctsdownloads.CosmicCameraControls'

prefix := '/usr'
bin-src := 'target/release/' + name
bin-dst := prefix + '/bin/' + name
desktop-src := 'res/' + appid + '.desktop'
desktop-dst := prefix + '/share/applications/' + appid + '.desktop'
icon-src := 'res/' + appid + '.svg'
icon-dst := prefix + '/share/icons/hicolor/scalable/apps/' + appid + '.svg'

user-prefix := env_var('HOME') + '/.local'
user-bin-dst := user-prefix + '/bin/' + name
user-desktop-dst := user-prefix + '/share/applications/' + appid + '.desktop'
user-icon-dst := user-prefix + '/share/icons/hicolor/scalable/apps/' + appid + '.svg'

default: build-release

# Run the app from a terminal for debugging
run:
    cargo run

# Build optimized release binary
build-release:
    cargo build --release

# Build and install system-wide (requires sudo, writes to /usr)
install: build-release
    sudo install -Dm0755 {{bin-src}} {{bin-dst}}
    sudo install -Dm0644 {{desktop-src}} {{desktop-dst}}
    sudo install -Dm0644 {{icon-src}} {{icon-dst}}

# Build and install user-local (no sudo; works on Fedora/Atomic/Pop/CachyOS/NixOS regardless of whether ~/.local/bin is in PATH)
install-user: build-release
    install -Dm0755 {{bin-src}} {{user-bin-dst}}
    install -Dm0644 {{desktop-src}} {{user-desktop-dst}}
    install -Dm0644 {{icon-src}} {{user-icon-dst}}
    sed -i 's|^Exec=.*|Exec={{user-bin-dst}}|' {{user-desktop-dst}}

# Remove system-wide install
uninstall:
    sudo rm -f {{bin-dst}}
    sudo rm -f {{desktop-dst}}
    sudo rm -f {{icon-dst}}

# Remove user-local install
uninstall-user:
    rm -f {{user-bin-dst}}
    rm -f {{user-desktop-dst}}
    rm -f {{user-icon-dst}}
