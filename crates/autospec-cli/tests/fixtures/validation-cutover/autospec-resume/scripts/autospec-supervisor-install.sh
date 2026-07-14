#!/usr/bin/env bash
launchd
systemd
@reboot
case "${1:-}" in
  install) ;;
  uninstall) ;;
  status) ;;
esac
