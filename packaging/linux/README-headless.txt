EdgeSwarm Node — Linux Headless Mode
====================================

The Linux package contains both the graphical desktop application and
the headless/server node.

Desktop/laptop:
    Launch: edgeswarm-node

Server/headless:
    Binary: edgeswarm-node-headless
    Unit:   edgeswarm-node-headless@USER.service

The headless service is deliberately NOT enabled automatically when
the package is installed.

Before enabling headless mode create:

    /etc/edgeswarm-node/USER/node.env
    /etc/edgeswarm-node/USER/wallet-password

The wallet-password file must be protected from other users.

Then explicitly enable the service for the intended provider account:

    sudo systemctl enable --now edgeswarm-node-headless@USER.service

Do not run the graphical node and the headless service simultaneously
for the same provider/device identity.
