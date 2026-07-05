# Issue 02: SSH password authentication fails

## Symptom

Logging in as `anjie` via SSH password fails. The server logs show PAM/crypto
errors.

## Cause

The Debian image has a broken PAM or password-hash configuration, so `sshd`
cannot verify passwords.

## Fix

Use public-key authentication instead.

On the board:

```bash
mkdir -p /home/anjie/.ssh
chmod 700 /home/anjie/.ssh
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOWpLBjiasmXninyxyZI/MAENwbr+zb2v3fnKmowuZCh 25418@QuteWin" \
    > /home/anjie/.ssh/authorized_keys
chmod 600 /home/anjie/.ssh/authorized_keys
chown -R anjie:anjie /home/anjie/.ssh
```

From Windows:

```powershell
ssh -o StrictHostKeyChecking=no -o PasswordAuthentication=no `
    -o PreferredAuthentications=publickey anjie@192.168.100.2
```

## Verification

SSH login succeeds without a password prompt.
