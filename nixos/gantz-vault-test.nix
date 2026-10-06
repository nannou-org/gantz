# The vault service offline: it starts, reads the DNS config, writes a private
# ticket, holds its directory while it serves, stops cleanly and keeps its
# identity.
{ module }:
{
  name = "gantz-vault";

  nodes.machine = {
    imports = [ module ];
    services.gantz-vault = {
      enable = true;
      openFirewall = true;
    };
    # Unreachable, but the vault must be able to read it.
    networking.nameservers = [ "192.0.2.53" ];
  };

  testScript = ''
    import re

    def vault_ids():
        log = machine.succeed("journalctl -u gantz-vault")
        return re.findall(r"vault (\w+) in /var/lib/gantz-vault", log)

    machine.wait_for_unit("gantz-vault.service")
    machine.wait_for_file("/var/lib/gantz-vault/ticket")
    machine.succeed("grep -q '^gantzvault' /var/lib/gantz-vault/ticket")
    machine.succeed("test \"$(stat -c '%U %a' /var/lib/gantz-vault)\" = 'gantz-vault 700'")
    machine.succeed("test \"$(stat -c '%U %a' /var/lib/gantz-vault/ticket)\" = 'gantz-vault 600'")
    machine.wait_until_succeeds("ss -Huln sport = :7447 | grep -q .")
    machine.succeed("iptables -S nixos-fw | grep -q -- '-p udp .*--dport 7447'")
    machine.fail("journalctl -u gantz-vault | grep -q 'Failed to read the system.s DNS config'")

    with subtest("the vault holds its directory while it serves"):
        out = machine.fail("gantz-vault devices 2>&1")
        assert "in use" in out, out

    with subtest("a stop ends the vault cleanly"):
        machine.succeed("systemctl stop gantz-vault")
        assert machine.succeed("systemctl show -P Result gantz-vault").strip() == "success"
        machine.succeed("journalctl -u gantz-vault | grep -q 'INFO stopped'")
        assert machine.succeed("gantz-vault devices").strip() == "No paired devices."

    with subtest("a restart keeps the identity of the vault"):
        machine.succeed("systemctl start gantz-vault")
        machine.wait_until_succeeds("test $(journalctl -u gantz-vault | grep -c 'vault .* in /var/lib/gantz-vault') -eq 2")
        first, second = vault_ids()
        assert first == second, (first, second)
  '';
}
