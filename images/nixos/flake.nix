# The NixOS image for Sherpa's `nixos_linux` lab nodes. Build it and import it as described in
# the NixOS page of the Sherpa docs:
#
#   nix build .#nixosConfigurations.sherpa-node.config.system.build.images.qemu
#
# flake.lock pins nixpkgs, so a build is reproducible; `nix flake update` moves to the newest
# 26.05 packages.
{
  description = "NixOS image for Sherpa lab nodes";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { nixpkgs, ... }: {
    nixosConfigurations.sherpa-node = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [ ./configuration.nix ];
    };
  };
}
