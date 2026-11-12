# room-fetch-browser (S1-02, S1-10; design §3.1; R1.2, R1.4): the room-fetch contract with
# Playwright's Firefox, so each page also yields a sealed screenshot and its DOM after scripts
# ran. Through the job's egress proxy only. The contract, and why Firefox (the proxy refuses ECH,
# which this Chromium sends regardless of flags), are in room-fetch-browser/fetch.py.
#   docker build -f crates/agentrix-sandbox/images/room-fetch-browser.Dockerfile \
#     -t room-fetch-browser:dev crates/agentrix-sandbox/images/room-fetch-browser
# The runner starts it with gVisor, a read-only root, user 65534 and a tmpfs /tmp; the browser's
# own sandbox cannot start there, so the room is the sandbox.
FROM mcr.microsoft.com/playwright/python:v1.48.0-noble
# The base image carries the browsers (firefox-1465, chromium-1140) but not the Python package; pin the
# package to the same release so it finds them.
RUN pip install --no-cache-dir --break-system-packages playwright==1.48.0
COPY fetch.py /opt/room/fetch.py
RUN chmod 0644 /opt/room/fetch.py && chmod -R a+rX /ms-playwright
USER 65534:65534
ENV HOME=/tmp PLAYWRIGHT_BROWSERS_PATH=/ms-playwright PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["python3", "/opt/room/fetch.py"]
