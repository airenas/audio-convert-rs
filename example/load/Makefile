vus?=1
it?=10
len?=50
format?=m4a
############################################################################
.data:
	mkdir -p $@

.data/1.wav: | .data
	@echo "Generating 1.wav with length ${len} seconds"
	@ffmpeg -f lavfi -i sine=frequency=1000:duration=${len} -ac 1 -ar 22050 $@	

.data/audio_convert.proto: ../../protos/audio_convert.proto | .data
	cp $^ $@ 

build: .data/1.wav .data/audio_convert.proto
	docker build -t k6-load-test .
.PHONY: build	
############################################################################
start/docker:
	docker run --rm -it --name audio_convert -p 50051:50051 airenas/audio-convert-rs:dev
.PHONY: start/docker	
###############################################################################
run: build
	docker run --rm -it --add-host=host.docker.internal:host-gateway -e K6_VUS=$(vus) \
		-e K6_ITERATIONS=$(it) -e FORMAT=$(format) k6-load-test
.PHONY: run
###############################################################################
clean:
	rm -rf .data

