# Communication

This category covers email clients, instant messaging, and video conferencing applications.

## Terminal Chat Clients

### Irssi

[Irssi](https://irssi.org/) is a modular text mode chat client. It comes with IRC support built in.

#### Installation

```nix
environment.systemPackages = [ pkgs.irssi ];
```

#### Verified Usage

```bash
# Start Irssi
irssi

# Inside irssi:
# /connect irc.libera.chat  - Connect to an IRC server
# /join #mychannel          - Join a channel
# /part #mychannel          - Leave a channel
# /quit                     - Quit irssi
```

### WeeChat

[WeeChat](https://weechat.org/) is a fast, light and extensible chat client, with a text-based user interface.

#### Installation

```nix
environment.systemPackages = [ pkgs.weechat ];
```

#### Verified Usage

```bash
# Start WeeChat
weechat

# Inside weechat:
# /server add libera irc.libera.chat
# /connect libera
# /join #mychannel
# /part #mychannel
# /quit
```
