# A gantz vault as a NixOS service. A vault syncs all the named graphs of a
# user between their devices. See `gantz vault --help`.
#
# The vault makes its identity and pairing secret on its first start, in
# `dataDir`. Run the vault commands as root through `gantz-vault`.
# `gantz-vault ticket` asks the running vault for the ticket that links a
# device. The journal records that a ticket was issued, but never the ticket.
# `devices` and `revoke` refuse to run while the vault serves, so stop the
# service before `gantz-vault devices`.
{
  config,
  lib,
  pkgs,
  utils,
  ...
}:
let
  inherit (lib)
    escapeShellArg
    mkEnableOption
    mkIf
    mkOption
    mkPackageOption
    optionals
    types
    ;

  cfg = config.services.gantz-vault;
  gantz = "${cfg.package}/bin/gantz";

  # A vault command as the service user, on the vault directory.
  admin = pkgs.writeShellScriptBin "gantz-vault" ''
    exec ${pkgs.util-linux}/bin/runuser -u ${escapeShellArg cfg.user} -- \
      ${gantz} vault "$@" --dir ${escapeShellArg cfg.dataDir}
  '';

  hardening = {
    CapabilityBoundingSet = "";
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    NoNewPrivileges = true;
    PrivateDevices = true;
    PrivateTmp = true;
    ProcSubset = "pid";
    ProtectClock = true;
    ProtectControlGroups = true;
    ProtectHome = true;
    ProtectHostname = true;
    ProtectKernelLogs = true;
    ProtectKernelModules = true;
    ProtectKernelTunables = true;
    ProtectProc = "invisible";
    ProtectSystem = "strict";
    # iroh watches the network interfaces over netlink.
    RestrictAddressFamilies = [
      "AF_INET"
      "AF_INET6"
      "AF_NETLINK"
      "AF_UNIX"
    ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
    ];
    UMask = "0077";
  };
in
{
  options.services.gantz-vault = {
    enable = mkEnableOption "a gantz vault, which syncs all named graphs between the devices of a user";

    package = mkPackageOption pkgs "gantz" { };

    dataDir = mkOption {
      type = types.path;
      default = "/var/lib/gantz-vault";
      description = ''
        The vault directory. It holds every synced graph with its whole
        history, the identity and pairing secret of the vault, and the socket
        that hands out the link ticket. Only the service user can enter it.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 7447;
      description = ''
        The UDP port that the vault binds. A fixed port keeps old tickets
        valid across restarts.
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Open `port` for UDP on all interfaces, so that devices can connect
        directly. Devices that cannot connect directly use a relay.
      '';
    };

    relay = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "https://relay.example.com";
      description = ''
        The URL of a self-hosted iroh relay, used instead of the default
        public infrastructure. `null` uses the default.
      '';
    };

    user = mkOption {
      type = types.str;
      default = "gantz-vault";
      description = ''
        The user that runs the vault and owns `dataDir`. The module creates
        the default user.
      '';
    };

    group = mkOption {
      type = types.str;
      default = "gantz-vault";
      description = ''
        The group of `dataDir`. The module creates the default group.
      '';
    };
  };

  config = mkIf cfg.enable {
    users.users = mkIf (cfg.user == "gantz-vault") {
      gantz-vault = {
        isSystemUser = true;
        group = cfg.group;
        description = "gantz vault";
      };
    };
    users.groups = mkIf (cfg.group == "gantz-vault") { gantz-vault = { }; };

    systemd.tmpfiles.settings.gantz-vault.${cfg.dataDir}.d = {
      user = cfg.user;
      group = cfg.group;
      mode = "0700";
    };

    networking.firewall.allowedUDPPorts = mkIf cfg.openFirewall [ cfg.port ];

    environment.systemPackages = [ admin ];

    systemd.services.gantz-vault = {
      description = "gantz vault";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = hardening // {
        ExecStart = utils.escapeSystemdExecArgs (
          [
            gantz
            "vault"
            "serve"
            "--dir"
            cfg.dataDir
            "--port"
            (toString cfg.port)
          ]
          ++ optionals (cfg.relay != null) [
            "--relay"
            cfg.relay
          ]
        );
        User = cfg.user;
        Group = cfg.group;
        ReadWritePaths = [ cfg.dataDir ];
        Restart = "on-failure";
        RestartSec = 10;
      };
    };
  };
}
