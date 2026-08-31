EdgeSwarm Node — Linux Headless Provider
=========================================

Linux is supported as a headless/provider node only.

First-time setup:
    edgeswarm-node-setup

Setup signs in to your provider account, verifies MFA,
creates or recovers this devices wallet identity, saves
the authenticated session, prepares the protected systemd
credential, and starts the provider-node service.

Service:
    edgeswarm-node-headless@USER.service

Status:
    systemctl status edgeswarm-node-headless@USER.service

Logs:
    journalctl -u edgeswarm-node-headless@USER.service

Live console:
    edgeswarm-node-status

Press Ctrl+C to close the console. The EdgeSwarm node continues running under systemd.
