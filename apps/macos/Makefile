APP_NAME   = bagent
BUILD_DIR  = .build/release
BUNDLE     = $(APP_NAME).app
REPO_ROOT  = ../..
DAEMON_DEV = $(REPO_ROOT)/target/debug/bagentd
DAEMON_REL = $(REPO_ROOT)/target/release/bagentd
# Stable signing identity: ad-hoc ("-") signatures use a per-build cdhash as the
# designated requirement, so every rebuild invalidates TCC grants (Accessibility,
# Screen Recording). A real identity keys the grant to identifier+team instead.
# Falls back to ad-hoc when no identity is available.
SIGN_ID   := $(shell security find-identity -v -p codesigning 2>/dev/null | grep -o '"Apple Development: [^"]*"' | head -1)
ifeq ($(SIGN_ID),)
SIGN_ID   := -
endif

.PHONY: run build daemon bundle clean whatsapp-bridge-install

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
	  unload_generate() { \
	    [ -n "$$1" ] || return 0; \
	    curl -sS --max-time 10 http://127.0.0.1:11434/api/generate \
	      -H 'Content-Type: application/json' \
	      -d "$$(printf '{"model":"%s","prompt":"","stream":false,"keep_alive":0}' "$$1")" \
	      >/dev/null 2>&1 || true; \
	  }; \
	  unload_embed() { \
	    curl -sS --max-time 10 http://127.0.0.1:11434/api/embeddings \
	      -H 'Content-Type: application/json' \
	      -d "$$(printf '{"model":"%s","prompt":"","keep_alive":0}' "$$1")" \
	      >/dev/null 2>&1 || true; \
	  }; \
	  model=$$(defaults read sk.bagent.app bagent.model 2>/dev/null || defaults read bagent bagent.model 2>/dev/null || echo qwen2.5:7b); \
	  classifier=$$(defaults read sk.bagent.app bagent.classifier_model 2>/dev/null || defaults read bagent bagent.classifier_model 2>/dev/null || echo qwen3:0.6b); \
	  unload_generate "$$model"; \
	  unload_generate "$$classifier"; \
	  unload_generate qwen2.5vl:7b; \
	  unload_embed bge-m3:latest; \
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
  <key>NSScreenCaptureUsageDescription</key> <string>bagent captures your screen on demand to answer questions about what is displayed. Screenshots are processed in memory and never saved to disk.</string>\n\
</dict></plist>\n' > "$(BUNDLE)/Contents/Info.plist"
	codesign --force --sign $(SIGN_ID) "$(BUNDLE)"
	@echo "✓  $(BUNDLE) ready — open $(BUNDLE) or drag to Applications"

clean:
	rm -rf .build $(BUNDLE)
	cargo clean --manifest-path $(REPO_ROOT)/Cargo.toml

# Install the WhatsApp Web bridge Node dependencies.
# Requires Node.js ≥ 18. Chromium/Puppeteer will be downloaded on first run (~150 MB).
# Run once before connecting WhatsApp for the first time.
whatsapp-bridge-install:
	cd $(REPO_ROOT)/crates/connectors/whatsapp/bridge && npm install
