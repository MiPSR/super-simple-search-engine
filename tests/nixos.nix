{ self, pkgs }:
{
  name = "ssse-container";
  nodes.machine = { pkgs, ... }: {
    imports = [ self.nixosModules.container ];
    services.ssseContainer.insecureCookie = true;
    environment.systemPackages = [ pkgs.curl pkgs.python3 ];
    virtualisation.memorySize = 1536;
    virtualisation.diskSize = 4096;
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("container@ssse.service")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:3000/api/stats")
    machine.succeed("curl -fsS http://127.0.0.1:3000/ | grep 'Super Simple Search Engine'")
    machine.succeed("curl -fsS http://127.0.0.1:3000/js/app.js | grep -E 'loadResults|function render'")
    machine.succeed("test -f /var/lib/ssse-container/ssse.db")
    machine.succeed("nixos-container run ssse -- ssse crawl status --socket /run/ssse/ssse.sock")
    # Password initialization uses the same restricted local protocol as the CLI.
    machine.succeed("nixos-container run ssse -- ${pkgs.python3}/bin/python3 -c 'import socket,json; s=socket.socket(socket.AF_UNIX); s.connect(\"/run/ssse/ssse.sock\"); s.sendall((json.dumps(dict(command=\"password_change\",username=\"admin\",password=\"vm-secret\"))+\"\\n\").encode()); assert json.loads(s.recv(4096))[\"ok\"]'")
    machine.succeed("curl -fsS -c /tmp/cookies -H 'Origin: http://127.0.0.1:3000' -H 'Content-Type: application/json' --data '{\"username\":\"admin\",\"password\":\"vm-secret\"}' http://127.0.0.1:3000/api/admin/login")
    machine.succeed("curl -fsS -b /tmp/cookies -X PUT -H 'Origin: http://127.0.0.1:3000' -H 'Content-Type: application/json' --data '{\"max_depth\":0}' http://127.0.0.1:3000/api/admin/settings")
    machine.succeed("nixos-container run ssse -- systemctl restart ssse")
    machine.wait_until_succeeds("curl -fsS -b /tmp/cookies http://127.0.0.1:3000/api/admin/settings | grep '\"max_depth\":0'")
    machine.succeed("nixos-container run ssse -- stat -c '%a' /run/ssse/ssse.sock | grep 600")
  '';
}
