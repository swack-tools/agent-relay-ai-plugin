# Relay Map hub. Built from a context made by build-image.sh: dist/ (front end),
# bin/relay-map-<os>-<arch> (static hub and collectors), relay-send-via-ssh.py.
FROM alpine:3.22
RUN apk add --no-cache openssh-client python3 curl \
 && adduser -D -u 10001 -h /home/relay relay
COPY dist/ /app/dist/
COPY bin/ /app/bin/
COPY relay-send-via-ssh.py /app/relay-send-via-ssh.py
RUN cp /app/bin/relay-map-linux-$(uname -m) /usr/local/bin/relay-map
USER 10001
WORKDIR /home/relay
EXPOSE 7878
ENTRYPOINT ["/usr/local/bin/relay-map", "serve", "--config", "/etc/relay-map/relay-map.toml"]
