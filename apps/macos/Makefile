APP_NAME   = bagent
BUILD_DIR  = .build/release
BUNDLE     = $(APP_NAME).app
REPO_ROOT  = ../..
DAEMON_DEV = $(REPO_ROOT)/target/debug/bagentd
DAEMON_REL = $(REPO_ROOT)/target/release/bagentd

.PHONY: run build daemon bundle clean

# Dev run: builds daemon first, then launches Swift app (which auto-finds the debug binary)
run: daemon-dev
	swift run

# Build daemon debug binary
daemon-dev:
	cargo build --manifest-path $(REPO_ROOT)/Cargo.toml

# Build daemon release binary
daemon:
	cargo build --release --manifest-path $(REPO_ROOT)/Cargo.toml

# Release Swift build
build:
	swift build -c release

# Bundle: Swift + daemon release binary
bundle: build daemon
	rm -rf $(BUNDLE)
	mkdir -p "$(BUNDLE)/Contents/MacOS"
	cp "$(BUILD_DIR)/$(APP_NAME)" "$(BUNDLE)/Contents/MacOS/"
	cp "$(DAEMON_REL)" "$(BUNDLE)/Contents/MacOS/"
	@printf '<?xml version="1.0" encoding="UTF-8"?>\n\
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n\
<plist version="1.0"><dict>\n\
  <key>CFBundleIdentifier</key>  <string>sk.bagent.app</string>\n\
  <key>CFBundleName</key>        <string>bagent</string>\n\
  <key>CFBundleExecutable</key>  <string>bagent</string>\n\
  <key>CFBundleVersion</key>     <string>0.1.0</string>\n\
  <key>NSPrincipalClass</key>    <string>NSApplication</string>\n\
  <key>LSUIElement</key>         <true/>\n\
  <key>NSHighResolutionCapable</key> <true/>\n\
</dict></plist>\n' > "$(BUNDLE)/Contents/Info.plist"
	codesign --force --sign - "$(BUNDLE)"
	@echo "✓  $(BUNDLE) ready — open $(BUNDLE) or drag to Applications"

clean:
	rm -rf .build $(BUNDLE)
	cargo clean --manifest-path $(REPO_ROOT)/Cargo.toml
