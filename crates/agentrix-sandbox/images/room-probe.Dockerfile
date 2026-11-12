# room-probe (test bench only): a tiny room for the runner's live checks (S1-01, S1-03) and the
# gauntlet's attack 9 measurements. Not used by the product.
#   docker build -f crates/agentrix-sandbox/images/room-probe.Dockerfile \
#     -t room-probe:dev crates/agentrix-sandbox/images/room-probe
FROM alpine:3.20
RUN apk add --no-cache curl jq
COPY probe.sh /probe.sh
RUN chmod 0755 /probe.sh
USER 65534:65534
ENV HOME=/tmp
ENTRYPOINT ["/probe.sh"]
