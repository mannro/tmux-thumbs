#!/usr/bin/env bash
set -Eeu -o pipefail

# Removing the binary to make this script idempotent
rm -rf target/release/thumbs

clear

cat << EOF

  █████                                                       █████    █████                                 █████
 ░░███                                                       ░░███    ░░███                                 ░░███
 ███████   █████████████   █████ ████ █████ █████            ███████   ░███████   █████ ████ █████████████   ░███████   █████
░░░███░   ░░███░░███░░███ ░░███ ░███ ░░███ ░░███  ██████████░░░███░    ░███░░███ ░░███ ░███ ░░███░░███░░███  ░███░░███ ███░░
  ░███     ░███ ░███ ░███  ░███ ░███  ░░░█████░  ░░░░░░░░░░   ░███     ░███ ░███  ░███ ░███  ░███ ░███ ░███  ░███ ░███░░█████
  ░███ ███ ░███ ░███ ░███  ░███ ░███   ███░░░███              ░███ ███ ░███ ░███  ░███ ░███  ░███ ░███ ░███  ░███ ░███ ░░░░███
  ░░█████  █████░███ █████ ░░████████ █████ █████             ░░█████  ████ █████ ░░████████ █████░███ █████ ████████  ██████
   ░░░░░  ░░░░░ ░░░ ░░░░░   ░░░░░░░░ ░░░░░ ░░░░░               ░░░░░  ░░░░ ░░░░░   ░░░░░░░░ ░░░░░ ░░░ ░░░░░ ░░░░░░░░  ░░░░░░

EOF


if [ "${1:-install}" == "update" ]; then

cat << EOF
  ⚠️  UPDATE! ⚠️

  It looks like you got a new version of tmux-thumbs repository but
  the binary version is not in sync.

  We are going to proceed with the new installation.

  Do you want to continue?

  Press any key to continue...
EOF

else

cat << EOF
  It looks like this is the first time you are executing tmux-thumbs
  because the binary is not present. We are going to proceed with the
  installation.

  Do you want to continue?

  Press any key to continue...
EOF

fi

read -rs -n 1

# This fork always compiles: upstream release binaries lack the fork's options.
if [ -x "$(command -v cargo)" ]; then
  cargo=(cargo)
elif [ -x "$(command -v mise)" ]; then
  cargo=(mise exec rust -- cargo)
else
  echo '❌ Rust is not installed!'
  exit 1
fi

echo '  Compiling tmux-thumbs, be patient:'
"${cargo[@]}" build --release --target-dir=target

cat << EOF
  Installation complete! 💯

  Press any key to close this pane...
EOF

read -rs -n 1
