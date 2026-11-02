{ modulesPath, pkgs, ... }:
{
  imports = [ (modulesPath + "/profiles/qemu-guest.nix") ];

  system.stateVersion = "26.05";

  # Sherpa configures each node on first boot from a cloud-init (NoCloud) disk: accounts, SSH
  # keys, host keys, hostname and the management address.
  services.cloud-init = {
    enable = true;
    network.enable = true;
    settings = {
      # NixOS's sshd generates the host keys on first boot (the image has none), and Sherpa pins
      # them once the node is ready. cloud-init leaves them alone: deleting them would leave
      # sshd serving keys that no longer exist on disk and change on the next restart.
      ssh_deletekeys = false;
      ssh_genkeytypes = [ ];
      # The helper that prints host keys to the console is not packaged on NixOS.
      ssh.emit_keys_to_console = false;
    };
  };
  networking.useNetworkd = true;
  networking.useDHCP = false;

  # Name interfaces eth0, eth1, ... as the manifest does.
  networking.usePredictableInterfaceNames = false;

  # cloud-init creates the sherpa and lab-owner accounts at boot, each with its own group, and
  # puts them in wheel.
  users.mutableUsers = true;
  security.loginDefs.settings.USERGROUPS_ENAB = "yes";
  security.sudo.wheelNeedsPassword = false;

  # Password login is allowed, as on Sherpa's other Linux nodes. cloud-init cannot change the
  # generated sshd_config, so it is set here.
  services.openssh = {
    enable = true;
    settings.PasswordAuthentication = true;
    settings.PermitRootLogin = "no";
  };

  boot.kernelParams = [ "console=ttyS0,115200" ];
  boot.growPartition = true;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  # cloud-init on the PATH: Sherpa's SSH provisioner waits on `cloud-init status --wait`.
  environment.systemPackages = with pkgs; [ cloud-init vim git curl ];
}
