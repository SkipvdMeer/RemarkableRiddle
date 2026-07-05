#!/usr/bin/env bash
# Build for the reMarkable 2 and copy everything over.
# Usage: ./deploy.sh [root@<tablet-ip>]
#   USB cable:  ./deploy.sh                (10.11.99.1)
#   Wi-Fi:      ./deploy.sh root@192.168.x.x
set -euo pipefail
cd "$(dirname "$0")"

HOST="${1:-root@10.11.99.1}"
TARGET=armv7-unknown-linux-musleabihf

export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo zigbuild --release --target "$TARGET"

ssh "$HOST" "mkdir -p /home/root/riddle"
scp "target/$TARGET/release/riddle" "$HOST:/home/root/riddle/"
# Never overwrite the tablet's config — it may hold the API key and tuning.
if ssh "$HOST" "test ! -e /home/root/riddle/riddle.toml"; then
  scp riddle.toml "$HOST:/home/root/riddle/"
fi

echo
echo "Deployed. On the tablet:"
echo "  ssh $HOST"
echo "  cd /home/root/riddle"
echo "  export OPENAI_API_KEY=sk-..."
echo "  ./riddle test-draw     # first time: check orientation"
echo "  ./riddle test-erase    # first time: check the vanishing"
echo "  ./riddle               # the full diary"
