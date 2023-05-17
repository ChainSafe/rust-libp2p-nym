#!/bin/bash
# Script to build the docker image.
# NOTE: `1.1.12` is the nym version we are using in the file explicitly.
# So this version reflects this. Ideally, this could be replaced by an
# official nym docker container should that exist.
set -euo pipefail
IFS=$'\n\t'
set -xf
docker build -t 'chainsafe/nym:1.1.12' -f ./Dockerfile.nym .
