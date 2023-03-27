use std::{
    collections::BTreeMap,
    fmt::Display,
    process::{Command, Output},
    rc::Rc,
};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{eyre, Result};
use nix::unistd::AccessFlags;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// A Nix Garbage Collection Root.
pub struct GCRoot {
    /// Location of the symlink.
    pub path: Rc<Utf8Path>,
    /// Where the symlink points to.
    pub target: Rc<Utf8Path>,
}

impl GCRoot {
    /// Returns Some(path) to where the profile should be if this gcroot file name
    /// fits the naming scheme of a profile generation, None otherwise.
    pub fn get_profile_path(&self) -> Option<&str> {
        self.get_profile_gen()
            .map(|_| self.path.as_str().rsplitn(3, '-').skip(2).next().unwrap())
    }

    /// Returns Some(generation number) of this profile generation if this gcroot file
    /// name fits the naming scheme of a profile generation, None otherwise.
    pub fn get_profile_gen(&self) -> Option<u64> {
        let Some(file_name) = self.path.file_name() else { return None };
        if file_name.chars().filter(|x| *x == '-').count() >= 2 {
            let mut iter = file_name.rsplitn(3, '-');
            if iter.next().unwrap() == "link" {
                let generation = iter.next().unwrap().parse::<u64>();
                generation.ok()
            } else {
                None
            }
        } else {
            None
        }
    }

    /// If the gcroot can be deleted.
    /// Doesn't check if the gcroot is an active profile.
    pub fn deletable(&self) -> bool {
        !self.path.starts_with("/run")
            && !self.path.starts_with("/proc")
            && Self::can_delete_file(&self.path)
    }

    fn can_delete_file(path: &Utf8Path) -> bool {
        path.parent()
            .map(|parent| nix::unistd::access(parent.as_str(), AccessFlags::W_OK).is_ok())
            .unwrap_or(false)
    }
}

impl Display for GCRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {}", self.path, self.target)
    }
}

#[derive(Debug)]
/// A Nix profile with its generations.
pub struct Profile {
    /// Path to the symlink pointing at the active profile generation.
    pub path: Utf8PathBuf,
    /// Some(genertion number) of the active generation.
    /// None if we don't know the active generation e.g. couldn't read the symlink.
    pub active_generation: Option<u64>,
    pub generations: BTreeMap<u64, GCRoot>,
}

impl Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)?;
        let digits = 1 + self
            .generations
            .keys()
            .max()
            .and_then(|m| m.checked_ilog10())
            .unwrap_or(0) as usize;
        for (id, generation) in self.generations.iter().rev() {
            writeln!(f)?;
            if self.active_generation == Some(*id) {
                write!(f, "> {: >digits$} -> {}", id, generation.target)?;
            } else {
                write!(f, "  {: >digits$} -> {}", id, generation.target)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
/// A collection of Nix Garbage Collection Roots.
pub struct GCRoots {
    profiles: Vec<Profile>,
    /// GCRoots that don't belong to any profile.
    standalone: Vec<GCRoot>,
}

impl GCRoots {
    /// Discovers GCRoots by running the nix-store command and parsing the output.
    pub fn from_nix_store_command() -> Result<Self> {
        let output = Command::new("nix-store")
            .args(["--gc", "--print-roots"])
            .output()?;
        let gcroots = Self::parse_nix_store_gc_output(output)?;
        Ok(Self::group_gcroots(gcroots)?)
    }

    fn parse_nix_store_gc_output(output: Output) -> Result<Vec<GCRoot>> {
        let output_bytes = output
            .status
            .success()
            .then_some(output.stdout)
            .ok_or_else(|| {
                eyre!(
                    "\"nix-store --gc --print-roots\" exited with code {}",
                    output.status
                )
            })?;
        let output_string = String::from_utf8(output_bytes)?;
        let result = output_string
            .lines()
            .filter_map(Self::parse_nix_store_gc_line)
            .collect();
        Ok(result)
    }

    fn parse_nix_store_gc_line(line: &str) -> Option<GCRoot> {
        let (path, target) = line
            .rsplit_once(" -> ")
            .expect("\"nix-store --gc --print-roots\" line containing \" -> \"");

        if !path.starts_with("/proc") && !(path.starts_with('{') && path.ends_with('}')) {
            Some(GCRoot {
                path: Utf8PathBuf::from(path).into(),
                target: Utf8PathBuf::from(target).into(),
            })
        } else {
            None
        }
    }

    fn group_gcroots(gcroots: Vec<GCRoot>) -> Result<Self> {
        let mut profiles = Self::create_profiles(&gcroots)?;
        let mut standalone = Self::populate_profiles(gcroots, &mut profiles);
        standalone.sort_unstable();
        Ok(GCRoots {
            profiles,
            standalone,
        })
    }

    fn create_profiles(gcroots: &[GCRoot]) -> Result<Vec<Profile>> {
        let mut profile_paths = gcroots
            .iter()
            .map(|gcroot| gcroot.get_profile_path())
            .filter_map(|o| o.map(Utf8PathBuf::from))
            .filter(|path| path.is_symlink())
            .collect::<Vec<_>>();
        profile_paths.sort_unstable();
        profile_paths.dedup();
        let mut profiles = Vec::with_capacity(profile_paths.len());
        for path in profile_paths {
            let active_generation = Self::read_active_gen(&path)?;
            profiles.push(Profile {
                path,
                active_generation,
                generations: BTreeMap::new(),
            })
        }
        profiles.sort_unstable_by(|p1, p2| p1.path.cmp(&p2.path));
        Ok(profiles)
    }

    fn read_active_gen(profile_path: &Utf8Path) -> Result<Option<u64>> {
        if Self::can_read_file(profile_path) {
            let link = profile_path.read_link_utf8()?;
            let Some(name) = link.file_name() else { return Ok(None) };
            let Some(generation) = name.rsplitn(3, '-').skip(1).next() else { return Ok(None) };
            Ok(generation.parse().ok())
        } else {
            Ok(None)
        }
    }

    fn can_read_file(path: &Utf8Path) -> bool {
        nix::unistd::access(path.as_str(), AccessFlags::R_OK).is_ok()
    }

    fn populate_profiles(gcroots: Vec<GCRoot>, profiles: &mut [Profile]) -> Vec<GCRoot> {
        let mut standalone = Vec::new();
        gcroots.into_iter().for_each(|gcroot| {
            let search = gcroot
                .get_profile_path()
                .and_then(|profile_path| {
                    profiles
                        .iter_mut()
                        .find(|p| p.path.as_str() == profile_path)
                })
                .and_then(|profile| {
                    gcroot
                        .get_profile_gen()
                        .map(|generation| (profile, generation))
                });
            if let Some((profile, generation)) = search {
                profile.generations.insert(generation, gcroot.into());
            } else {
                standalone.push(gcroot);
            };
        });
        standalone
    }
}

impl Display for GCRoots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, profile) in self.profiles.iter().enumerate() {
            if index != 0 {
                writeln!(f)?;
            }
            if index + 1 < self.profiles.len() {
                writeln!(f, "{}", profile)?;
            } else {
                write!(f, "{}", profile)?;
            }
        }
        if self.standalone.len() > 0 {
            write!(f, "\n\n")?;
        }
        for (index, standalone) in self.standalone.iter().enumerate() {
            if index != 0 {
                writeln!(f)?;
            }
            write!(f, "{}", standalone)?;
        }
        Ok(())
    }
}
