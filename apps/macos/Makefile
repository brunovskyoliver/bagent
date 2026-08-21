APP_NAME   = bagent
BUILD_DIR  = .build/release
BUNDLE     = $(APP_NAME).app
REPO_ROOT  = ../..
DAEMON_DEV = $(REPO_ROOT)/target/debug/bagentd
DAEMON_REL = $(REPO_ROOT)/target/release/bagentd
# Stable signing identity: ad-hoc ("-") signatures use a per-build cdhash as the
# designated requirement, so every rebuild invalidates TCC grants (Accessibility,
# Screen Recording). A real identity keys the grant to identifier+team instead.
# Falls back to ad-hoc for the developer-only `make run` path.
EXPECTED_SIGN_ID := Apple Development: obrunovsky7@gmail.com (D63PW2838J)
SIGN_ID   := $(shell security find-identity -v -p codesigning 2>/dev/null | grep -F '"$(EXPECTED_SIGN_ID)"' | head -1 | sed -n 's/^[^\"]*"\([^\"]*\)".*$$/\1/p')
ifeq ($(SIGN_ID),)
SIGN_ID   := -
endif
ICON_OUTPUT := .build/generated-icon

.PHONY: run build daemon daemon-acceptance bundle bundle-acceptance assemble-bundle prepare-icon require-signing clean whatsapp-bridge-install

# Dev run: builds daemon first, then the Swift app. The Info.plist is embedded in
# the binary (see Package.swift) so screen-capture TCC works without bundling. Ad-hoc
# sign with a stable identifier so the mic grant is attributed consistently.
run: daemon-dev
	swift build
	codesign --force --sign $(SIGN_ID) --identifier sk.bagent.app .build/debug/bagent
	@cleanup() { \
	  pid_file="$$HOME/Library/Application Support/bagent/daemon.pid"; \
	  if [ -f "$$pid_file" ]; then \
	    pid=$$(cat "$$pid_file" 2>/dev/null || true); \
	    if [ -n "$$pid" ]; then kill -TERM "$$pid" 2>/dev/null || true; fi; \
	  fi; \
	  launchctl bootout "gui/$$(id -u)/com.bagent.basert" >/dev/null 2>&1 || true; \
	  true; \
	}; \
	trap cleanup EXIT INT TERM; \
	.build/debug/bagent

# Build daemon debug binary
daemon-dev:
	cargo build --manifest-path $(REPO_ROOT)/Cargo.toml

# Build daemon release binary
daemon:
	cargo build --release --manifest-path $(REPO_ROOT)/Cargo.toml

# Explicitly compiled Stage 8 acceptance daemon. The fixture routes still
# require BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1 at daemon startup.
daemon-acceptance:
	cargo build --release --manifest-path $(REPO_ROOT)/Cargo.toml -p bagentd --features stage8-acceptance

# Release Swift build
build:
	swift build -c release

# Bundle: Swift + daemon release binary
bundle: build daemon
	$(MAKE) require-signing
	$(MAKE) assemble-bundle
	@echo "✓  $(BUNDLE) ready — open $(BUNDLE) or drag to Applications"

# Signed-app acceptance bundle. This is never used by the ordinary bundle
# target, so production/default builds contain no fixture controls.
bundle-acceptance: build daemon-acceptance
	$(MAKE) require-signing
	$(MAKE) assemble-bundle
	@echo "✓  $(BUNDLE) Stage 8 acceptance build ready"

assemble-bundle:
	rm -rf $(BUNDLE)
	mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	cp "$(BUILD_DIR)/$(APP_NAME)" "$(BUNDLE)/Contents/MacOS/"
	cp "$(DAEMON_REL)" "$(BUNDLE)/Contents/MacOS/"
	$(MAKE) prepare-icon
	cp "$(ICON_OUTPUT)/AppIcon.icns" "$(BUNDLE)/Contents/Resources/"
	cp "$(ICON_OUTPUT)/bagent-permission.png" "$(BUNDLE)/Contents/Resources/"
	@printf '<?xml version="1.0" encoding="UTF-8"?>\n\
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n\
<plist version="1.0"><dict>\n\
  <key>CFBundleIdentifier</key>  <string>sk.bagent.app</string>\n\
  <key>CFBundleName</key>        <string>bagent</string>\n\
  <key>CFBundleExecutable</key>  <string>bagent</string>\n\
  <key>CFBundlePackageType</key> <string>APPL</string>\n\
  <key>CFBundleIconFile</key>     <string>AppIcon</string>\n\
  <key>CFBundleVersion</key>     <string>0.1.0</string>\n\
  <key>NSPrincipalClass</key>    <string>NSApplication</string>\n\
  <key>LSUIElement</key>         <true/>\n\
  <key>NSHighResolutionCapable</key> <true/>\n\
  <key>NSScreenCaptureUsageDescription</key> <string>bagent captures your screen on demand to answer questions about what is displayed. Screenshots are processed in memory and never saved to disk.</string>\n\
  </dict></plist>\n' > "$(BUNDLE)/Contents/Info.plist"
	codesign --force --sign "$(SIGN_ID)" --identifier sk.bagent.app.daemon "$(BUNDLE)/Contents/MacOS/bagentd"
	codesign --force --sign "$(SIGN_ID)" --identifier sk.bagent.app "$(BUNDLE)/Contents/MacOS/bagent"
	codesign --force --sign "$(SIGN_ID)" --identifier sk.bagent.app "$(BUNDLE)"
	codesign --verify --deep --strict "$(BUNDLE)"

prepare-icon:
	$(REPO_ROOT)/scripts/acceptance/prepare-app-icon.sh "$(ICON_OUTPUT)"

require-signing:
	@security find-identity -v -p codesigning 2>/dev/null | grep -Fq '"$(EXPECTED_SIGN_ID)"' || { echo "required Apple Development identity unavailable; refusing ad-hoc Stage 7C bundle" >&2; exit 2; }

clean:
	rm -rf .build $(BUNDLE)
	cargo clean --manifest-path $(REPO_ROOT)/Cargo.toml

# Install the WhatsApp Web bridge Node dependencies.
# Requires Node.js ≥ 18. Chromium/Puppeteer will be downloaded on first run (~150 MB).
# Run once before connecting WhatsApp for the first time.
whatsapp-bridge-install:
	cd $(REPO_ROOT)/crates/connectors/whatsapp/bridge && npm install
