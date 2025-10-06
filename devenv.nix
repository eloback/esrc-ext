{
  pkgs,
  config,
  lib,
  ...
}:
let
  cfg = config.custom;
in
{
  imports = [
    {
      packages = [
        # NATS
        pkgs.natscli
      ];
      processes.nats-server = {
        exec = "${lib.getExe pkgs.nats-server} -js -DV -sd .devenv/state/nats";
        process-compose = {
          ready_log_line = "Server is ready";
        };
      };
    }
  ];
  config = {
    custom = {
      common.project_name = "esrc-ext";
      rust.enable = false;
    };
    languages.rust = {
      channel = "stable";
      enable = true;
    };
    git-hooks.hooks = {
      rustfmt = {
        enable = true;
        files = "\.rs$";
      };
      clippy = {
        enable = true;
        settings = {
          allFeatures = true;
        };
      };
    };

  };
}