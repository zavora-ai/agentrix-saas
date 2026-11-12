# room-fetch (S1-02; design §3.1; R1.2): fetches the URLs control gives it through the job's
# egress proxy and records each answer. The contract is in room-fetch/fetch.sh.
#   docker build -f crates/agentrix-sandbox/images/room-fetch.Dockerfile \
#     -t room-fetch:dev crates/agentrix-sandbox/images/room-fetch
FROM alpine:3.20
RUN apk add --no-cache curl jq coreutils
COPY fetch.sh /opt/room/fetch.sh
RUN chmod 0755 /opt/room/fetch.sh
USER 65534:65534
ENV HOME=/tmp
ENTRYPOINT ["/opt/room/fetch.sh"]
