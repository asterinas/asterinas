{ pkgs, ... }:
let
  irssi_exp = pkgs.writeTextFile {
    name = "irssi.exp";
    text = ''
      #!/usr/bin/expect -f
      set timeout 30
      spawn irssi
      expect {
          "irssi" { }
          timeout { puts "FAIL: irssi did not start"; exit 1 }
      }
      send "/connect irc.libera.chat\r"
      expect {
          "established" { puts "OK: connected" }
          timeout       { puts "FAIL: timeout"; exit 1 }
      }
      send "/quit\r"
      expect eof
      puts "PASS"
    '';
  };
  weechat_exp = pkgs.writeTextFile {
    name = "weechat.exp";
    text = ''
      #!/usr/bin/expect -f
      set timeout 30
      spawn weechat
      expect {
          "WeeChat" { }
          timeout   { puts "FAIL: WeeChat did not start"; exit 1 }
      }
      send "/server add libera irc.libera.chat/6667 -notls\r"
      send "/connect libera\r"
      expect {
          "connected" { puts "OK: connected" }
          timeout     { puts "FAIL: timeout"; exit 1 }
      }
      send "/quit\r"
      expect eof
      puts "PASS"
    '';
  };
in {
  environment.systemPackages = with pkgs; [ expect irssi weechat ];

  system.activationScripts.testFixtures = ''
    ln -sfT ${irssi_exp} /tmp/irssi.exp
    ln -sfT ${weechat_exp} /tmp/weechat.exp
  '';
}
