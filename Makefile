# hotty-rs. Toolchain from mise.toml: `mise install` first, or a rustup
# toolchain of at least the workspace's rust-version.
CARGO ?= cargo

# The hotty repository (SPEC.md, SDK.md, the vectors): HOTTY_DIR, or a
# checkout next to this one.
HOTTY_DIR ?= $(abspath $(patsubst %/SPEC.md,%,$(firstword $(wildcard ../hotty/SPEC.md ../../hotty/main/SPEC.md))))
export HOTTY_DIR

.PHONY: check fmt build test clippy doc minimal vectors

check: fmt build test clippy doc minimal   ## the gate

fmt:   ## fails when rustfmt would change files
	$(CARGO) fmt --all --check

build:
	$(CARGO) build --workspace --all-targets

test:   ## the tests, the conformance vectors among them
	$(CARGO) test --workspace -q

clippy:
	$(CARGO) clippy --workspace --all-targets -q -- -D warnings

doc:   ## the documentation, with no broken links
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps -q

minimal:   ## the wire layer alone: no zlib, no terminal
	$(CARGO) test -p hotty --no-default-features -q

vectors:   ## the conformance vectors, from a checkout of neuroplastio/hotty (HOTTY_DIR)
	@test -n "$(HOTTY_DIR)" || { echo "no hotty checkout: set HOTTY_DIR, or clone neuroplastio/hotty next to this repository"; exit 1; }
	cp $(HOTTY_DIR)/conformance/vectors.json crates/hotty/tests/testdata/vectors.json
