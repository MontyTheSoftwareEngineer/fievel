self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.fievel;
  settingsFormat = pkgs.formats.toml { };
in
{
  options.services.fievel = {
    enable = lib.mkEnableOption "Fievel keyboard and mouse control";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "fievel.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The Fievel package to use.";
    };

    users = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = ''
        Users who run Fievel. They join the `input` and `uinput` groups, and the
        service starts with their graphical session. These groups let any process
        those users run read keystrokes and inject input.
      '';
    };

    settings = lib.mkOption {
      inherit (settingsFormat) type;
      default = { };
      example = {
        mode = "toggle";
        keys.free_mouse = "f3";
      };
      description = ''
        Fievel configuration, used by every user in place of their
        {file}`~/.config/fievel/fievel.config`. The file is read-only, so the
        config screen cannot save changes to it. Leave this empty to let each
        user manage their own file.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    hardware.uinput.enable = true;
    environment.systemPackages = [ cfg.package ];
    users.users = lib.genAttrs cfg.users (_: {
      extraGroups = [
        "input"
        "uinput"
      ];
    });

    systemd.user.services.fievel = {
      description = "Fievel keyboard and mouse control";
      after = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      wantedBy = [ "graphical-session.target" ];
      # User units exist for everyone, so skip users without device access.
      unitConfig.ConditionGroup = "input";
      # Keep the session's PATH, which the config screen needs to open a browser.
      environment.PATH = lib.mkForce null;
      serviceConfig = {
        ExecStart =
          lib.getExe cfg.package
          + lib.optionalString (
            cfg.settings != { }
          ) " --config ${settingsFormat.generate "fievel.config" cfg.settings}";
        Restart = "always";
        RestartSec = 2;
      };
    };
  };
}
