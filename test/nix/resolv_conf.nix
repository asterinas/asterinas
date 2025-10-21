{ stdenv, dnsServer }:
let
  host_resolv_conf = builtins.path {
    name = "host-resolv-conf";
    path = "/etc/resolv.conf";
  };
in stdenv.mkDerivation {
  name = "resolv-conf";
  buildCommand = ''
    RESOLV_CONF_FILE="$out/resolv.conf"
    mkdir -p $out

    is_host_resolve_conf_valid() {
      if [ ! -f "${host_resolv_conf}" ]; then
        return 1
      fi

      if grep -qE "nameserver\s+127\.0\.0\." "${host_resolv_conf}"; then
        return 1
      else
        return 0
      fi
    }

    if [ -n "${dnsServer}" ] && [ "${dnsServer}" != "none" ]; then
      echo "nameserver ${dnsServer}" > "$RESOLV_CONF_FILE"
    elif is_host_resolve_conf_valid; then
      cp ${host_resolv_conf} $RESOLV_CONF_FILE
      echo "resolv.conf is generated from the host's /etc/resolv.conf"
    else
      echo "Warning: the host's /etc/resolv.conf is not valid for the guest VM (containing lookback addresses)." >&2
      echo "Fall back to Cloudflare's public DNS servers (1.1.1.1)." >&2
      echo "Consider using the DNS_SERVER Makefile variable to specify DNS server explicitly." >&2
      echo "For example: make DNS_SERVER=\"192.168.1.1\"" >&2
      echo "nameserver 1.1.1.1" > "$RESOLV_CONF_FILE"
    fi
  '';
}
