# room-render (S1-02; design §3.1; R6.2): Chromium renders the candidate's pages, served over the
# room's own loopback, in a sealed room with no network, and saves a screenshot per page.
# Contract: room-render/render.py.
#   docker build -f crates/agentrix-sandbox/images/room-render.Dockerfile \
#     -t room-render:dev crates/agentrix-sandbox/images/room-render
# The runner starts it with gVisor, a read-only root, user 65534, a tmpfs /tmp and --network none;
# Chromium's own sandbox cannot start there, so the room is the sandbox (--no-sandbox).
FROM mcr.microsoft.com/playwright/python:v1.48.0-noble
# The base image ships the browsers but not the Python package.
RUN pip install --no-cache-dir --break-system-packages playwright==1.48.0
COPY render.py /opt/room/render.py
RUN chmod 0644 /opt/room/render.py && chmod -R a+rX /ms-playwright
USER 65534:65534
ENV HOME=/tmp PLAYWRIGHT_BROWSERS_PATH=/ms-playwright PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["python3", "/opt/room/render.py"]
