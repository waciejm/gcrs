use std::{collections::BTreeMap, process::Command};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{eyre, Result};
use nix::unistd::AccessFlags;

pub trait GCRoot {
    fn deletable(&self) -> bool;

    fn path(&self) -> &str;

    fn delete(&self) -> Result<()> {
        Ok(std::fs::remove_file(self.path())?)
    }
}

#[derive(Debug)]
pub struct StandaloneGCRoot {
    pub path: Utf8PathBuf,
    pub target: Utf8PathBuf,
}

impl GCRoot for StandaloneGCRoot {
    fn deletable(&self) -> bool {
        !self.path.starts_with("/run")
            && !self.path.starts_with("/proc")
            && can_delete_file(&self.path)
    }

    fn path(&self) -> &str {
        self.path.as_str()
    }
}

#[derive(Debug)]
pub struct ProfileGenerationGCRoot {
    pub path: Utf8PathBuf,
    pub target: Utf8PathBuf,
    pub generation: u64,
    pub is_active: Option<bool>,
}

impl GCRoot for ProfileGenerationGCRoot {
    fn deletable(&self) -> bool {
        self.is_active.map(|x| !x).unwrap_or(false) && can_delete_file(&self.path)
    }

    fn path(&self) -> &str {
        self.path.as_str()
    }
}

#[derive(Debug)]
pub struct Profile {
    pub path: Utf8PathBuf,
    pub generations: BTreeMap<u64, ProfileGenerationGCRoot>,
    pub active_generation: Option<u64>,
}

#[derive(Debug)]
pub struct GCRoots {
    profiles: Vec<Profile>,
    standalone: Vec<StandaloneGCRoot>,
}

impl GCRoots {
    pub fn from_nix_store_command() -> Result<Self> {
        let output = Command::new("nix-store")
            .args(["--gc", "--print-roots"])
            .output()?;
        let output_bytes = if output.status.success() {
            output.stdout
        } else {
            return Err(eyre!(
                "\"nix-store --gc --print-roots\" exited with code {}",
                output.status
            ));
        };
        let output_string = String::from_utf8(output_bytes)?;

        let gcroots = output_string
            .lines()
            .map(parse_nix_store_gc_line)
            .filter_map(|x| x)
            .collect::<Vec<_>>();

        let mut profile_paths = gcroots
            .iter()
            .map(|gcroot| get_profile_path(&gcroot.path))
            .filter_map(|o| o.map(Utf8PathBuf::from))
            .filter(|path| path.is_symlink())
            .collect::<Vec<_>>();
        profile_paths.sort_unstable();
        profile_paths.dedup();

        let mut profiles = Vec::with_capacity(profile_paths.len());
        let mut buf = Utf8PathBuf::with_capacity(100);
        for path in profile_paths {
            let active_generation = read_active_gen(&path, &mut buf)?;
            profiles.push(Profile {
                path,
                generations: BTreeMap::new(),
                active_generation,
            })
        }

        let mut standalone = Vec::<StandaloneGCRoot>::new();

        for gcroot in gcroots.into_iter() {
            if let Some(profile) = profiles
                .iter_mut()
                .find(|p| Some(p.path.as_str()) == get_profile_path(&gcroot.path))
            {
                let generation = get_profile_gen(&gcroot.path).unwrap();
                let is_active = if let Some(x) = profile.active_generation {
                    Some(generation == x)
                } else {
                    None
                };
                profile.generations.insert(
                    generation,
                    ProfileGenerationGCRoot {
                        path: gcroot.path,
                        target: gcroot.target,
                        generation,
                        is_active,
                    },
                );
            } else {
                standalone.push(gcroot);
            }
        }

        Ok(GCRoots {
            profiles,
            standalone,
        })
    }
}

fn can_delete_file(path: &Utf8Path) -> bool {
    path.parent()
        .map(|parent| nix::unistd::access(parent.as_str(), AccessFlags::W_OK).is_ok())
        .unwrap_or(false)
}

fn can_read_file(path: &Utf8Path) -> bool {
    nix::unistd::access(path.as_str(), AccessFlags::R_OK).is_ok()
}

fn parse_nix_store_gc_line(line: &str) -> Option<StandaloneGCRoot> {
    let (path, target) = line
        .rsplit_once(" -> ")
        .expect("\"nix-store --gc --print-roots\" line containing \" -> \"");

    if !path.starts_with("/proc") && !(path.starts_with('{') && path.ends_with('}')) {
        Some(StandaloneGCRoot {
            path: path.into(),
            target: target.into(),
        })
    } else {
        None
    }
}

fn get_profile_path(path: &Utf8Path) -> Option<&str> {
    get_profile_name(path).map(|_| path.as_str().rsplitn(3, '-').skip(2).next().unwrap())
}

fn get_profile_name(path: &Utf8Path) -> Option<&str> {
    is_profile_generation_path(path).map(|x| x.0)
}

fn get_profile_gen(path: &Utf8Path) -> Option<u64> {
    is_profile_generation_path(path).map(|x| x.1)
}

fn is_profile_generation_path(path: &Utf8Path) -> Option<(&str, u64)> {
    let Some(file_name) = path.file_name() else { return None };
    if file_name.chars().filter(|x| *x == '-').count() >= 2 {
        let mut iter = file_name.rsplitn(3, '-');
        if iter.next().unwrap() == "link" {
            let generation = iter.next().unwrap().parse::<u64>();
            if generation.is_ok() {
                Some((iter.next().unwrap(), generation.unwrap()))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    }
}

fn read_active_gen(profile_path: &Utf8Path, buf: &mut Utf8PathBuf) -> Result<Option<u64>> {
    let Some(profile_path_parent) = profile_path.parent() else { return Ok(None) };
    if can_read_file(profile_path) {
        let current = profile_path.read_link()?;
        let Some(current) = current.to_str() else { return Ok(None) };
        buf.clear();
        buf.push(profile_path_parent);
        buf.push(current);
        Ok(get_profile_gen(&buf))
    } else {
        Ok(None)
    }
}
