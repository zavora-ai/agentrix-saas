# room-build (S1-02; design §3.1): runs an old theme's build hook in a sealed room with no
# network and the room's caps. Contract: room-build/build.sh.
#   docker build -f crates/agentrix-sandbox/images/room-build.Dockerfile \
#     -t room-build:dev crates/agentrix-sandbox/images/room-build
FROM alpine:3.20
RUN apk add --no-cache jq coreutils tar gzip
COPY build.sh /opt/room/build.sh
RUN chmod 0755 /opt/room/build.sh
USER 65534:65534
ENV HOME=/tmp
ENTRYPOINT ["/opt/room/build.sh"]
