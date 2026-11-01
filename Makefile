# ntop — build / run / install. Needs a Rust toolchain (https://rustup.rs) or Docker for `docker-build`.
CARGO  ?= cargo
PREFIX ?= /usr/local
BIN     = target/release/ntop

.PHONY: build run test install uninstall clean docker-build

build:
	$(CARGO) build --release

run: build
	./$(BIN)

test:
	$(CARGO) test

install: build
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/ntop

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/ntop

clean:
	$(CARGO) clean

# Build a Linux binary without a local toolchain (output lands in target/release/).
docker-build:
	docker run --rm -v "$(CURDIR)":/src -w /src rust:1-slim-bookworm cargo build --release
