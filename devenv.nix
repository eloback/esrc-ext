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
    (
      _:
      let
        db_user = "nixus";
        db_pass = "postgres";
      in
      {
        custom.pgdatabase.enable = false;
        services = {
          postgres = {
            enable = lib.mkDefault true;
            initialDatabases = lib.mkDefault [
              { name = "dead_letter_db"; }
            ];
            initdbArgs = [
              "--locale=C"
              "--encoding=UTF8"
              "--username=${db_user}"
            ];
          };
        };
        env = {
          DATABASE_URL = lib.mkDefault "postgres://${config.env.PGUSER}:${config.env.PGPASSWORD}@${lib.escapeURL config.env.DEVENV_RUNTIME}%2fpostgres/dead_letter_db";
          PGUSER = lib.mkDefault db_user;
          PGPASSWORD = lib.mkDefault db_pass;
          PGDATABASE = lib.mkDefault cfg.common.project_name;
        };
      }
    )
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