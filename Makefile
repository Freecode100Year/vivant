VERSION ?= v$(shell cat VERSION)$(shell git diff --quiet 2>/dev/null || echo -dirty)
CARGO := VIVANT_VERSION=$(VERSION) cargo
# static Linux binaries for typical VPSes (needs cargo-zigbuild and zig)
LINUX_TARGETS := x86_64-unknown-linux-musl:amd64 aarch64-unknown-linux-musl:arm64 armv7-unknown-linux-musleabihf:armv7

.PHONY: build test run dist bench lint clean

build:
	$(CARGO) build --release --locked --bin vivant
	mkdir -p bin && cp target/release/vivant bin/vivant

test:
	cargo test --locked

lint:
	cargo fmt --check
	cargo clippy --locked --all-targets -- -D warnings
	shellcheck -S warning install.sh deploy/make-offline-bundle.sh deploy/offline-header.sh
	bash install.sh --print-unit | diff -u deploy/vivant.service -

run: build
	./bin/vivant -addr 127.0.0.1:2222 -host-key ./data/host_ed25519

# What the release workflow publishes for Linux, plus checksums.
dist:
	mkdir -p dist
	for t in $(LINUX_TARGETS); do \
		triple=$${t%%:*}; arch=$${t##*:}; \
		$(CARGO) zigbuild --release --locked --bin vivant --target $$triple && \
		cp target/$$triple/release/vivant dist/vivant-linux-$$arch || exit 1; \
	done
	cp install.sh dist/
	deploy/make-offline-bundle.sh dist $(VERSION)
	cd dist && sha256sum vivant-* install.sh > SHA256SUMS

# Load test against a local server with relaxed limits.
bench:
	cargo build --release --locked
	./target/release/vivant -addr 127.0.0.1:2299 -host-key ./data/bench_host -max-per-ip 100000 -max-tcp-per-ip 100000 \
		-conn-per-min 100000 -max-clients 100000 -msg-rate 100 -msg-burst 100 -read-rate 0 -tor-exits=false & \
	PID=$$!; sleep 1; ./target/release/vivant-bench -addr 127.0.0.1:2299 -clients 500 -senders 25 -rate 0.5 -duration 20s; kill $$PID

clean:
	cargo clean
	rm -rf bin dist
