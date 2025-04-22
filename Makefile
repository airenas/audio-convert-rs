-include Makefile.options
stream?=
#####################################################################################
## print usage information
help:
	@echo 'Usage:'
	@cat ${MAKEFILE_LIST} | grep -e "^## " -A 1 | grep -v '\-\-' | sed 's/^##//' | cut -f1 -d":" | \
		awk '{info=$$0; getline; print "  " $$0 ": " info;}' | column -t -s ':' | sort 
.PHONY: help
#####################################################################################
## run the server
run:
	RUST_LOG=TRACE,h2=INFO,tonic=INFO cargo run --bin audio-convert-rs -- 
.PHONY: run
###############################################################################
file?=1.wav
run/client:
	RUST_LOG=TRACE,h2=INFO,tonic=INFO cargo run --bin audio-convert-rs-cl -- -i $(file) ${stream}
.PHONY: run/client
###############################################################################
run/build: build/local
	RUST_LOG=DEBUG target/release/audio-convert-rs --
.PHONY: run/build
###############################################################################
## build the server
build/local: 
	cargo build --release
.PHONY: build/local
###############################################################################
## run unit tests
test/unit:
	RUST_LOG=DEBUG cargo test --no-fail-fast
.PHONY: test/unit
###############################################################################
## run lint
test/lint:
	@cargo clippy -V
	cargo clippy --all-targets --all-features -- -D warnings
.PHONY: test/lint	
###############################################################################
## clean all
clean:
	cargo clean
.PHONY: clean
