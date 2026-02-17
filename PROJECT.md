# Features

## Database
- [x] Application database schema implementation.

## Users
- [] Allow for user defined credential including passwords SSH public keys.
- [] Store user secret

## Logging
- [] Add more logging/tracing

## Authentication
- [] Implement Authentication/Authorization system

## Validate Command
- [x] Add a valiate command to validate the manifest

## Sherpa Server
- [] sherpad service systemd definition.

### API
- [] Implement API endpoints for all lab actions (up, destroy, etc).

## Sherpa Client

### CLI
- [] Update actions to utilise API endpoints.
- [] Use websockets to connect to Sherpa server.

## SSH
 - Get node SSH fingerprints and generate a `lab_known_hosts_file`
 - This can be sent to the client and added to the SSH config file so that we can
   stop ignoring SSH certs.

## TLS
- [] Provision tls certificates for lab nodes.
- [] Lets encrypt certificates for Sherpa application.

## Node Networking
- [] Isolated bridge/per host management outside of
  libvirt to include isolated bridges for container
  nodes.
- [] Point-to-Point UDP tunnels for VM nodes.
- [] Point-to-Point vETH pairs for container nodes.
- [] Public bridge implementation.

## SSH
- [] SSH jump host configuration.
- [] SSH tunneling for service forwarding.

## Install
- [] Install script generation.

## Node Implementations
- [] Nokia SR Linux container node.
- [] Unikernel node.

## Node Config Database Table
- [] Add the ability to track multiple node versions in the database.
- [] When importing a node image, the node_config should also be imported into the database.

## Testing
- [] There a many tests to add.


# Completed
