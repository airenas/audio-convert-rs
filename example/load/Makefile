vus?=1
it?=10

1.wav:
	ffmpeg -f lavfi -i sine=frequency=1000:duration=5 -ac 1 -ar 22050 $@

build: 1.wav
	docker build -t k6-load-test .

run: build
	docker run --rm -it --add-host=host.docker.internal:host-gateway -e K6_VUS=$(vus) -e K6_ITERATIONS=$(it) k6-load-test


