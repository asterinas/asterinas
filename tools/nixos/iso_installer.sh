#!@shell@

# SPDX-License-Identifier: MPL-2.0

set -e

echo "=== Asterinas NixOS Installer ==="
echo "Default paths:"
echo "  distro: @distroPath@"
echo "  kernel: @kernelPath@"
echo "  tools: @toolsPath@"
echo

if [ "$(tty)" != "/dev/hvc0" ]; then
    echo "Warning: The installer only runs on /dev/hvc0 console!"
    exit 0
fi

AUTO_INSTALL=${AUTO_INSTALL:-"@autoInstall@"}

get_user_input() {
    local prompt="$1"
    local default="$2"
    local var="$3"

    if [ "$AUTO_INSTALL" -eq 1 ]; then
        if [ -n "$default" ]; then
            eval "$var=\$default"
            echo "$prompt: $default (auto)"
        else
            eval "$var="
            echo "$prompt: (auto - using empty value)"
        fi
        return
    fi

    if [ -n "$default" ]; then
        read -p "$prompt [$default]: " input
        if [ -z "$input" ]; then
            eval "$var=\$default"
        else
            eval "$var=\$input"
        fi
    else
        read -p "$prompt: " input
        eval "$var=\$input"
    fi
}

get_user_input "Proceed with configuration? (y/n)" "y" confirm
if [ "$confirm" = "n" ] || [ "$confirm" = "N" ]; then
    echo "Installation cancelled!"
    exit 0
fi

export NIXOS_KERNEL=@kernelPath@
export NIXOS_STAGE_1_INIT=@toolsPath@/stage_1_init.sh

get_user_input "Install AsterNixOS on" "/dev/vda" DEVICE
if [ -z "$DEVICE" ] || [ ! -e "$DEVICE" ] || [ ! -b "$DEVICE" ]; then
    echo "Error: Invalid device $DEVICE"
    exit 1
fi
get_user_input "Warning: All data on $DEVICE will be erased! Continue? (y/n)" "y" confirm
if [ "$confirm" = "n" ] || [ "$confirm" = "N" ]; then
    echo "Installation cancelled!"
    exit 0
fi
sgdisk --zap-all $DEVICE
sync
partprobe $DEVICE

NIXOS_DIR=$(mktemp -d)
cp -r @distroPath@/* $NIXOS_DIR
chmod -R u+w $NIXOS_DIR
export DISTRO_DIR=$NIXOS_DIR

CONFIGURATION="$DISTRO_DIR/configuration.nix"
if [ -f "$CONFIGURATION" ]; then
    get_user_input "Do you want to edit the configuration? (y/n) " "n" edit_config
    if [ "$edit_config" = "y" ] || [ "$edit_config" = "Y" ]; then
        echo "Opening configuration file for editing..."
        echo "Press Enter to continue..."
        read

        if command -v vim >/dev/null 2>&1; then
            vim "$CONFIGURATION"
        elif command -v nano >/dev/null 2>&1; then
            nano "$CONFIGURATION"
        elif command -v vi >/dev/null 2>&1; then
            vi "$CONFIGURATION"
        else
            echo "=== Current configuration ==="
            cat "$CONFIGURATION"
            echo "============================"
            echo "No editor found. Configuration displayed above."
        fi

        echo "Configuration editing finished."
    fi
else
    echo "Error: Configuration file not found in @distroPath@"
    exit 1
fi

get_user_input "Proceed with installation? (y/n)" "y" confirm
if [ "$confirm" = "n" ] || [ "$confirm" = "N" ]; then
    echo "Installation cancelled!"
    exit 0
fi
@toolsPath@/install_asterinas.sh ${NIXOS_DIR} ${DEVICE}

get_user_input "Power off system? (y/n)" "y" poweroff_choice
if [ "$poweroff_choice" = "n" ] || [ "$poweroff_choice" = "N" ]; then
    echo "System will remain running!"
else
    poweroff
fi
