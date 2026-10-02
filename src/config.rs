use std::{ffi::OsString, path::PathBuf, time::Duration};

pub const HELP: &str = "wlpipes — XP 3D Pipes recreation for Wayland (not a screen locker)
Usage: wlpipes [options]
  -w, --windowed       Ordinary preview window
      --overlay        Fullscreen dismissible overlay (default)
      --pipes MODE     single (default), multiple (random 2..4), or a count 1..4
      --joints MODE    elbow (default), ball, mixed, or cycle
      --texture PATH  Textured surface using BMP/JPEG/TGA/PNG
      --default-texture PATH  Supplied default bitmap, also custom-file fallback
      --tessellation N XP quality level, 0..4 (default 0)
      --speed-level N  XP speed slider, 0..20 (default 16)
      --scene-length N Growth budget multiplier, 1..10 (default 1)
      --seed N         XP 32-bit seed before ten warm-up draws
      --max-seconds N  Exit after 1..86400 seconds, even without frame callbacks
  -h, --help           Show this help without connecting to Wayland
  -V, --version        Show version

XP layout adapts each output to its aspect ratio, with 15 nodes on the longer axis.
The grid, continuous orbit and teapot overrides have been removed.
One process shares an XP random stream across independent output scenes.
Solid is the default surface. Original texture artwork must be supplied locally.
Use pipes-lock for password-protected locking; this program only draws.
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joints {
    Elbow,
    Ball,
    Mixed,
    Cycle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pipes {
    Fixed(u8),
    Multiple,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub windowed: bool,
    pub tessellation: u8,
    pub joints: Joints,
    pub pipes: Pipes,
    pub speed: u8,
    pub scene_length: u8,
    pub texture: Option<PathBuf>,
    pub default_texture: Option<PathBuf>,
    pub seed: Option<u32>,
    pub lifetime: Option<Duration>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            windowed: false,
            tessellation: 0,
            joints: Joints::Elbow,
            pipes: Pipes::Fixed(1),
            speed: 16,
            scene_length: 1,
            texture: None,
            default_texture: None,
            seed: None,
            lifetime: None,
        }
    }
}

pub enum Action {
    Run(Config),
    Help,
    Version,
}
impl Config {
    pub fn textured(&self) -> bool {
        self.texture.is_some() || self.default_texture.is_some()
    }

    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
        let mut cfg = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let arg = arg.to_str().ok_or("options must be UTF-8")?;
            match arg {
                "-h" | "--help" => return Ok(Action::Help),
                "-V" | "--version" => return Ok(Action::Version),
                "-w" | "--windowed" => cfg.windowed = true,
                "--overlay" => cfg.windowed = false,
                "--texture" | "--default-texture" => {
                    let path =
                        PathBuf::from(args.next().ok_or_else(|| format!("{arg} needs a path"))?);
                    if arg == "--texture" {
                        cfg.texture = Some(path);
                    } else {
                        cfg.default_texture = Some(path);
                    }
                }
                "--pipes" | "--joints" | "--tessellation" | "--speed-level" | "--scene-length"
                | "--seed" | "--max-seconds" => {
                    let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
                    let value = value.to_str().ok_or("values must be UTF-8")?;
                    match arg {
                        "--pipes" => {
                            cfg.pipes = match value {
                                "single" => Pipes::Fixed(1),
                                "multiple" => Pipes::Multiple,
                                _ => Pipes::Fixed(number(value, 1, 4)?),
                            }
                        }
                        "--joints" => {
                            cfg.joints = match value {
                                "elbow" => Joints::Elbow,
                                "ball" => Joints::Ball,
                                "mixed" => Joints::Mixed,
                                "cycle" => Joints::Cycle,
                                _ => {
                                    return Err("joints must be elbow, ball, mixed or cycle".into());
                                }
                            }
                        }
                        "--tessellation" => cfg.tessellation = number(value, 0, 4)?,
                        "--speed-level" => cfg.speed = number(value, 0, 20)?,
                        "--scene-length" => cfg.scene_length = number(value, 1, 10)?,
                        "--seed" => cfg.seed = Some(number(value, 0, u32::MAX)?),
                        "--max-seconds" => {
                            cfg.lifetime = Some(Duration::from_secs(number(value, 1, 86_400)?));
                        }
                        _ => unreachable!(),
                    }
                }
                "--speed" => {
                    return Err("--speed milliseconds was removed; use --speed-level 0..20".into());
                }
                "--grid" | "--teapot" | "--rotate" | "--max-runs" | "--max-length" => {
                    return Err(format!(
                        "{arg} was removed: XP defines layout, joints and base growth budgets"
                    ));
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        Ok(Action::Run(cfg))
    }
}

fn number<T: std::str::FromStr + PartialOrd + Copy + std::fmt::Display>(
    value: &str,
    min: T,
    max: T,
) -> Result<T, String> {
    value
        .parse::<T>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("expected an integer in {min}..={max}, got {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Action, String> {
        Config::parse(args.iter().map(OsString::from))
    }
    #[test]
    fn texture_paths_are_explicit_and_need_not_be_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![
            b'/', b't', b'm', b'p', b'/', 0xff, b'.', b'b', b'm', b'p',
        ]);
        let Action::Run(cfg) = Config::parse([OsString::from("--texture"), path.clone()]).unwrap()
        else {
            panic!()
        };
        assert!(cfg.textured());
        assert_eq!(cfg.texture.unwrap().into_os_string(), path);
        let Action::Run(cfg) = parse(&["--default-texture", "default.bmp"]).unwrap() else {
            panic!()
        };
        assert!(cfg.textured());
        assert!(parse(&["--texture"]).is_err());
        assert!(!Config::default().textured());
    }

    #[test]
    fn settings_defaults_bounds_and_removed_overrides() {
        let Action::Run(cfg) = parse(&[]).unwrap() else {
            panic!()
        };
        assert_eq!(cfg.tessellation, 0);
        assert_eq!(cfg.speed, 16);
        assert_eq!(cfg.scene_length, 1);
        for length in 1..=10 {
            let Action::Run(config) = parse(&["--scene-length", &length.to_string()]).unwrap()
            else {
                panic!()
            };
            assert_eq!(config.scene_length, length);
            assert_eq!(config.speed, 16);
        }
        for level in 0..=20 {
            assert!(parse(&["--speed-level", &level.to_string()]).is_ok());
        }
        assert_eq!(cfg.joints, Joints::Elbow);
        assert_eq!(cfg.pipes, Pipes::Fixed(1));
        for (value, expected) in [
            ("single", Pipes::Fixed(1)),
            ("multiple", Pipes::Multiple),
            ("1", Pipes::Fixed(1)),
            ("2", Pipes::Fixed(2)),
            ("3", Pipes::Fixed(3)),
            ("4", Pipes::Fixed(4)),
        ] {
            let Action::Run(config) = parse(&["--pipes", value]).unwrap() else {
                panic!()
            };
            assert_eq!(config.pipes, expected);
        }
        for (value, expected) in [
            ("elbow", Joints::Elbow),
            ("ball", Joints::Ball),
            ("mixed", Joints::Mixed),
            ("cycle", Joints::Cycle),
        ] {
            let Action::Run(config) = parse(&["--joints", value]).unwrap() else {
                panic!()
            };
            assert_eq!(config.joints, expected);
        }
        for args in [
            vec!["--grid", "auto"],
            vec!["--rotate"],
            vec!["--teapot", "0"],
            vec!["--max-runs", "1"],
            vec!["--max-length", "1"],
            vec!["--tessellation", "5"],
            vec!["--joints", "teapot"],
            vec!["--pipes", "0"],
            vec!["--pipes", "5"],
            vec!["--pipes", "-1"],
            vec!["--pipes", "invalid"],
            vec!["--pipes"],
            vec!["--speed", "0"],
            vec!["--speed-level", "21"],
            vec!["--scene-length", "0"],
            vec!["--scene-length", "11"],
            vec!["--scene-length", "invalid"],
            vec!["--scene-length"],
            vec!["--seed", "-1"],
            vec!["--seed", "4294967296"],
            vec!["--max-seconds"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        let Action::Run(cfg) =
            parse(&["--windowed", "--tessellation", "4", "--seed", "4294967295"]).unwrap()
        else {
            panic!()
        };
        assert!(cfg.windowed);
        assert_eq!(cfg.seed, Some(u32::MAX));
        assert_eq!(cfg.tessellation, 4);
        assert!(matches!(parse(&["--help"]), Ok(Action::Help)));
        assert!(matches!(parse(&["--version"]), Ok(Action::Version)));
    }
}
