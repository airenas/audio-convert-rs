vus?=1
it?=10
len?=50

.data:
	mkdir -p $@

.data/1.wav: | .data
	@echo "Generating 1.wav with length ${len} seconds"
	@ffmpeg -f lavfi -i sine=frequency=1000:duration=${len} -ac 1 -ar 22050 $@	

.data/audio_convert.proto: ../../protos/audio_convert.proto | .data
	cp $^ $@ 

build: .data/1.wav .data/audio_convert.proto
	docker build -t k6-load-test .

run: build
	docker run --rm -it --add-host=host.docker.internal:host-gateway -e K6_VUS=$(vus) -e K6_ITERATIONS=$(it) k6-load-test

clean:
	rm -rf .data

