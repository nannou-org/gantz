# The vault service offline: it starts, reads the DNS config, hands its ticket
# to its own user alone and never logs it, holds its directory while it
# serves, stops cleanly, keeps its identity and logs what its filter allows.
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
    users.users.alice.isNormalUser = true;
  };

  nodes.quiet = {
    imports = [ module ];
    services.gantz-vault = {
      enable = true;
      logFilter = "warn";
    };
  };

  testScript = ''
    import re

    def vault_ids():
        log = machine.succeed("journalctl -u gantz-vault")
        return re.findall(r"vault (\w+) in /var/lib/gantz-vault", log)

    start_all()
    machine.wait_for_unit("gantz-vault.service")
    machine.wait_until_succeeds("ss -Huln sport = :7447 | grep -q .")
    machine.succeed("test \"$(stat -c '%U %a' /var/lib/gantz-vault)\" = 'gantz-vault 700'")
    machine.succeed("iptables -S nixos-fw | grep -q -- '-p udp .*--dport 7447'")
    machine.fail("journalctl -u gantz-vault | grep -q 'Failed to read the system.s DNS config'")

    with subtest("the journal stamps the lines, so the vault does not"):
        machine.succeed("journalctl -u gantz-vault -o cat | grep -q '^ *INFO gantz '")
        machine.fail("journalctl -u gantz-vault -o cat | grep -qE '^[0-9]{4}-'")

    with subtest("the vault hands its ticket to its own user"):
        ticket = machine.wait_until_succeeds("gantz-vault ticket").strip()
        assert ticket.startswith("gantzvault"), ticket
        machine.succeed("test \"$(stat -c '%U %a' /var/lib/gantz-vault/ticket.sock)\" = 'gantz-vault 600'")
        machine.succeed("journalctl -u gantz-vault | grep -q 'issued a link ticket'")

    with subtest("no other user can ask for the ticket"):
        gantz = machine.succeed("grep -o '/nix/store/[^ ]*/bin/gantz' $(command -v gantz-vault)").strip()
        out = machine.fail(f"sudo -u alice {gantz} vault ticket --dir /var/lib/gantz-vault 2>&1")
        assert "Only the user that runs the vault" in out, out

    with subtest("the ticket is never logged or left on disk"):
        machine.fail("journalctl -u gantz-vault | grep -q gantzvault")
        machine.succeed("test ! -e /var/lib/gantz-vault/ticket")

    with subtest("the vault holds its directory while it serves"):
        out = machine.fail("gantz-vault devices 2>&1")
        assert "in use" in out, out

    with subtest("a stop ends the vault cleanly"):
        machine.succeed("systemctl stop gantz-vault")
        assert machine.succeed("systemctl show -P Result gantz-vault").strip() == "success"
        machine.succeed("journalctl -u gantz-vault | grep -q 'INFO stopped'")
        machine.succeed("test ! -e /var/lib/gantz-vault/ticket.sock")
        out = machine.fail("gantz-vault ticket 2>&1")
        assert "not running" in out, out
        assert machine.succeed("gantz-vault devices").strip() == "No paired devices."

    with subtest("a restart keeps the identity and deletes an old ticket file"):
        machine.succeed("echo gantzvault-old > /var/lib/gantz-vault/ticket")
        machine.succeed("systemctl start gantz-vault")
        machine.wait_until_succeeds("test $(journalctl -u gantz-vault | grep -c 'vault .* in /var/lib/gantz-vault') -eq 2")
        first, second = vault_ids()
        assert first == second, (first, second)
        machine.wait_until_succeeds("test ! -e /var/lib/gantz-vault/ticket")
        machine.succeed("journalctl -u gantz-vault | grep -q 'deleted /var/lib/gantz-vault/ticket'")

    with subtest("the log filter decides what the vault logs"):
        quiet.wait_for_unit("gantz-vault.service")
        quiet.wait_until_succeeds("gantz-vault ticket")
        env = quiet.succeed("systemctl show -P Environment gantz-vault")
        assert "RUST_LOG=warn" in env, env
        quiet.fail("journalctl -u gantz-vault -o cat | grep -q INFO")
  '';
}
