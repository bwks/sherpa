# Features

## Database
- Application database schema implementation.

## Users
- Allow for user defined credential including passwords
  and SSH public keys.

## Sherpa Server
- sherpad service systemd definition.

### API
- Implement API endpoints for all lab actions (up, destroy, etc).

## Sherpa Client

### CLI
- Update actions to utilise API endpoints.
- Use websockets to connect to Sherpa server.

## TLS
- Provision tls certificates for lab nodes.
- Lets encrypt certificates for Sherpa application.

## Node Networking
- Isolated bridge/per host management outside of
  libvirt to include isolated bridges for container
  nodes.
- Point-to-Point UDP tunnels for VM nodes.
- Point-to-Point vETH pairs for container nodes.
- Public bridge implementation.

## SSH
- SSH jump host configuration.
- SSH tunneling for service forwarding.

## Install
- Install script generation.

## Node Implementations
- Nokia SR Linux container node.
- Unikernel node.

## Testing
- There a many tests to add.


# Completed
