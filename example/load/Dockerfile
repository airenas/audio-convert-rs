FROM grafana/k6:latest

WORKDIR /app

COPY load.js .
COPY 1.wav .
COPY audio_convert.proto ./definitions/

ENTRYPOINT ["k6", "run", "/app/load.js"]
