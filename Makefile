.PHONY: all clean install test release check uninstall

BINARY = mp4converter
INSTALL_DIR = $(HOME)/.local/bin
TARGET = target/release/$(BINARY)

all: check install

check:
	cargo fmt -- --check
	cargo clippy -- -D warnings
	cargo test

clean:
	cargo clean
	rm -f $(INSTALL_DIR)/$(BINARY)

test:
	cargo test

release:
	cargo build --release
	strip $(TARGET)

install: release
	@if [ ! -d $(INSTALL_DIR) ]; then \
		mkdir -p $(INSTALL_DIR); \
	fi
	cp $(TARGET) $(INSTALL_DIR)/
	@echo "Installed $(BINARY) to $(INSTALL_DIR)"
	@echo "Ensure $(INSTALL_DIR) is in your PATH"

uninstall:
	rm -f $(INSTALL_DIR)/$(BINARY)
