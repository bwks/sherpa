FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    tini \
    iproute2 \
    iputils-ping \
    net-tools \
    && rm -rf /var/lib/apt/lists/*

# Install iouyap from GNS3 PPA
RUN echo "deb http://ppa.launchpad.net/gns3/ppa/ubuntu focal main" > /etc/apt/sources.list.d/gns3.list \
    && apt-key adv --recv-keys --keyserver keyserver.ubuntu.com 9A2FD067A2E3EF7B \
    && apt-get update && apt-get install -y iouyap \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /iol

COPY iol.bin /iol/iol.bin
RUN chmod +x /iol/iol.bin

COPY config.txt /iol/config.txt
COPY entrypoint.sh /iol/entrypoint.sh
COPY start-iol.sh /iol/start-iol.sh
RUN chmod +x /iol/entrypoint.sh /iol/start-iol.sh

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/iol/entrypoint.sh"]
