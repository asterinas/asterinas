final: prev: {
  # Asterinas does not support RTM_GETROUTE, which Fastfetch has used since 2.50.0.
  # Pin to the last version that reads default routes from procfs instead.
  fastfetch = prev.fastfetch.overrideAttrs (_: {
    version = "2.49.0";
    src = prev.fetchFromGitHub {
      owner = "fastfetch-cli";
      repo = "fastfetch";
      tag = "2.49.0";
      hash = "sha256-M1/VThHWRB6MbmPpHcgaM3j07kmuj0RnjblKo54RatY=";
    };
  });
}
