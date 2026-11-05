//! Reine Logik der Linux-Seite: keine Systemaufrufe, deshalb auf jedem System testbar.

#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub point: String,
    /// Teilbaum des Dateisystems, der hier eingehaengt ist. Bei Bind-Mounts nicht `/`.
    pub root: String,
    pub fs: String,
    pub source: String,
    pub major: u32,
    pub minor: u32,
    pub opts: String,
}

/// `\040` steht in mountinfo fuer ein Leerzeichen.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            let d = &b[i + 1..i + 4];
            if d.iter().all(|c| (b'0'..=b'7').contains(c)) {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Liest `/proc/self/mountinfo`. Aufbau einer Zeile:
/// `36 35 98:0 /root /mnt rw,noatime master:1 - ext3 /dev/sda1 rw,errors=continue`
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut out = vec![];
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let l: Vec<&str> = left.split(' ').collect();
        let r: Vec<&str> = right.split(' ').collect();
        if l.len() < 6 || r.len() < 2 {
            continue;
        }
        let (major, minor) = l[2].split_once(':').unwrap_or(("0", "0"));
        out.push(Mount {
            point: unescape(l[4]),
            root: unescape(l[3]),
            fs: r[0].to_string(),
            source: unescape(r[1]),
            major: major.parse().unwrap_or(0),
            minor: minor.parse().unwrap_or(0),
            opts: l[5].to_string(),
        });
    }
    out
}

const PSEUDO_FS: &[&str] = &[
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "pstore",
    "bpf",
    "tracefs",
    "debugfs",
    "securityfs",
    "configfs",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "autofs",
    "binfmt_misc",
    "rpc_pipefs",
    "nsfs",
    "overlay",
    "squashfs",
    "ramfs",
    "efivarfs",
    "selinuxfs",
    "nfsd",
    "sunrpc",
    "rootfs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "fuse.snapfuse",
    "erofs",
];

const NETWORK_FS: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "fuse.rclone", "ceph", "glusterfs", "9p", "afs", "virtiofs"];

pub fn is_network_fs(fs: &str) -> bool {
    NETWORK_FS.contains(&fs)
}

/// Waehlt die Einhaengepunkte aus, die fuer Menschen Laufwerke sind.
/// Pseudo-Dateisysteme, Bind-Mounts und Paket-Abbilder (snap, Docker) gehoeren nicht dazu.
pub fn usable_mounts(all: Vec<Mount>) -> Vec<Mount> {
    let mut out: Vec<Mount> = vec![];
    for m in all {
        if PSEUDO_FS.contains(&m.fs.as_str()) {
            continue;
        }
        if m.root != "/" && m.fs != "btrfs" {
            continue;
        }
        let under = |p: &str| m.point == p || m.point.starts_with(&format!("{p}/"));
        if (under("/proc") || under("/sys") || under("/dev") || under("/run") || under("/snap") || under("/boot/efi") || under("/efi"))
            && !under("/run/media")
        {
            continue;
        }
        // Dasselbe Geraet mehrfach eingehaengt: Der kuerzeste Pfad steht fuer alle.
        if let Some(i) = out.iter().position(|o| o.source == m.source && o.root == m.root) {
            if m.point.len() < out[i].point.len() {
                out[i] = m;
            }
            continue;
        }
        out.push(m);
    }
    out.sort_by(|a, b| a.point.cmp(&b.point));
    out
}

/// Zu welchem Einhaengepunkt gehoert der Pfad? Der laengste passende gewinnt.
pub fn mount_of<'a>(mounts: &'a [Mount], path: &str) -> Option<&'a Mount> {
    mounts.iter().filter(|m| m.point == "/" || path == m.point || path.starts_with(&format!("{}/", m.point))).max_by_key(|m| m.point.len())
}

/// Prozent-Kodierung nach RFC 2396, wie sie die Papierkorb-Spezifikation fuer `Path=` verlangt.
pub fn percent_encode_path(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~' | b'!' | b'*' | b'\'' | b'(' | b')');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Inhalt einer `.trashinfo`-Datei.
pub fn trash_info(path_bytes: &[u8], date: &str) -> String {
    format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", percent_encode_path(path_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 28 0:21 / /sys rw,nosuid shared:7 - sysfs sysfs rw
23 28 0:22 / /proc rw,nosuid shared:14 - proc proc rw
28 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 28 8:3 / /home rw,noatime shared:30 - ext4 /dev/sda3 rw
41 28 8:17 / /mnt/mein\\040archiv rw,relatime shared:31 - ext4 /dev/sdb1 rw
42 28 8:17 /sub /srv/bind rw,relatime shared:31 - ext4 /dev/sdb1 rw
43 28 7:1 / /snap/core/1 ro,nodev shared:40 - squashfs /dev/loop1 ro
44 28 0:50 / /tmp rw shared:41 - tmpfs tmpfs rw
45 28 0:60 / /mnt/nas rw shared:42 - nfs4 nas:/film rw
46 28 8:1 / /boot/efi rw shared:43 - vfat /dev/sda1 rw
47 28 8:33 / /run/media/ich/USB rw shared:44 - exfat /dev/sdc1 rw
48 28 0:70 /@data /data rw shared:45 - btrfs /dev/sdd1 rw
";

    #[test]
    fn parses_mountinfo_with_escapes() {
        let m = parse_mountinfo(SAMPLE);
        assert_eq!(m.len(), 12);
        let a = m.iter().find(|x| x.source == "/dev/sdb1").unwrap();
        assert_eq!(a.point, "/mnt/mein archiv");
        assert_eq!((a.major, a.minor), (8, 17));
    }

    #[test]
    fn keeps_only_real_drives() {
        let points: Vec<String> = usable_mounts(parse_mountinfo(SAMPLE)).into_iter().map(|m| m.point).collect();
        assert_eq!(points, vec!["/", "/data", "/home", "/mnt/mein archiv", "/mnt/nas", "/run/media/ich/USB"]);
    }

    #[test]
    fn longest_mount_wins() {
        let all = parse_mountinfo(SAMPLE);
        assert_eq!(mount_of(&all, "/home/ich/x").unwrap().point, "/home");
        assert_eq!(mount_of(&all, "/homeoffice").unwrap().point, "/");
        assert_eq!(mount_of(&all, "/home").unwrap().point, "/home");
    }

    #[test]
    fn trash_info_is_encoded_as_the_spec_demands() {
        assert_eq!(percent_encode_path("/home/ich/Grüße 1.txt".as_bytes()), "/home/ich/Gr%C3%BC%C3%9Fe%201.txt");
        let t = trash_info(b"/a b", "2026-11-05T12:00:00");
        assert_eq!(t, "[Trash Info]\nPath=/a%20b\nDeletionDate=2026-11-05T12:00:00\n");
    }
}
