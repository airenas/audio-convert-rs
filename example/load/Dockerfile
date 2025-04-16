FROM grafana/k6:latest

WORKDIR /app

COPY load.js .
COPY .data/1.wav .
COPY .data/audio_convert.proto ./definitions/

ENTRYPOINT ["k6", "run", "/app/load.js"]
