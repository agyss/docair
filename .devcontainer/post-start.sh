#!/usr/bin/env bash
# Runs on every container start.
#
# Some host kernels (e.g. 7.x) have no legacy iptables `nat` table, so the
# Docker daemon that the docker-in-docker feature starts fails with
# "can't initialize iptables table `nat'". Switch to the nft backend and
# start the daemon again if it is not up.
set -euo pipefail

if ! sudo iptables-legacy -t nat -L -n >/dev/null 2>&1; then
  sudo update-alternatives --set iptables /usr/sbin/iptables-nft >/dev/null
  sudo update-alternatives --set ip6tables /usr/sbin/ip6tables-nft >/dev/null
fi

if ! docker info >/dev/null 2>&1; then
  /usr/local/share/docker-init.sh true
  for _ in $(seq 1 30); do docker info >/dev/null 2>&1 && break; sleep 1; done
fi

docker info --format 'docker {{.ServerVersion}} is up' || echo "docker is not up: see /tmp/dockerd.log"
