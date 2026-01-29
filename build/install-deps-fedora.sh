#!/bin/bash

echo "This is a sample script for installing dependencies on Fedora to build alicepi for aarch64 from x86_64."
echo "You need to run this script with root privileges."
set -e

if [ "$(uname -m)" != "x86_64" ]; then
  echo "This script is intended to be run on x86_64 host."
  exit 1
fi

# bail if not root
if [ "$EUID" -ne 0 ]; then
  echo "Please run as root"
  exit 1
fi

dnf install aarch64-linux-gnu-gcc.x86_64

sudo dnf --installroot=/usr/aarch64-linux-gnu/sys-root --releasever=42   --repofrompath=fedora-aarch64,https://download.fedoraproject.org/pub/fedora/linux/releases/42/Everything/aarch64/os/   --repofrompath=updates-aarch64,https://download.fedoraproject.org/pub/fedora/linux/updates/42/Everything/aarch64/   --enablerepo=fedora-aarch64 --enablerepo=updates-aarch64 --disablerepo='*'   --setopt=gpgcheck=1 --setopt=repo_gpgcheck=0   --setopt=gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-fedora-42-aarch64   install fedora-gpg-keys
sudo dnf --forcearch=aarch64 \
  --installroot=/usr/aarch64-linux-gnu/sys-root \
  --releasever=42 \
  --setopt=install_weak_deps=False \
  --repofrompath=fedora-aarch64,https://download.fedoraproject.org/pub/fedora/linux/releases/42/Everything/aarch64/os/ \
  --repofrompath=updates-aarch64,https://download.fedoraproject.org/pub/fedora/linux/updates/42/Everything/aarch64/ \
  --enablerepo=fedora-aarch64 \
  --enablerepo=updates-aarch64 \
  --disablerepo='*' \
  install \
  alsa-lib-devel \
  python3.11-devel \
  glibc-devel.aarch64 libstdc++-devel.aarch64 gcc-c++.aarch64

