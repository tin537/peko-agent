# Local developer workflow targets for peko-agent.
#
# Real-device testing intentionally runs locally (not in GitHub CI) so the
# OnePlus 6T (or any rooted Android device on USB) can be exercised
# without exposing it as a shared CI runner. CI handles build matrix +
# unit tests; on-device validation is `make device-test`.

.PHONY: help build test lint device-test device-test-phase1 \
        adb-check adb-push device-shell

PHASE ?= 1
ADB ?= adb
TARGET_DIR ?= /data/local/tmp/peko-agent-test

help:
	@echo "Local developer targets:"
	@echo "  build           cargo build --workspace"
	@echo "  test            cargo test --workspace --lib"
	@echo "  lint            cargo clippy --workspace -- -D warnings"
	@echo "  device-test     run on-device tests for current PHASE (default: $(PHASE))"
	@echo "                  e.g. 'make device-test PHASE=1'"
	@echo "  adb-check       confirm a device is connected and authorized"
	@echo "  device-shell    open an interactive root shell on the device"

build:
	cargo build --workspace

test:
	cargo test --workspace --lib

lint:
	cargo clippy --workspace -- -D warnings

adb-check:
	@$(ADB) get-state >/dev/null 2>&1 || (echo "no device connected via adb"; exit 1)
	@$(ADB) shell echo "device responsive" >/dev/null

device-shell: adb-check
	$(ADB) shell

device-test: adb-check
	@case "$(PHASE)" in \
	  1) $(MAKE) device-test-phase1 ;; \
	  2) $(MAKE) device-test-phase2 ;; \
	  *) echo "no device-test for PHASE=$(PHASE) yet"; exit 1 ;; \
	esac

device-test-phase1:
	@bash tests/device-test/phase1.sh

device-test-phase2:
	@bash tests/device-test/phase2.sh
