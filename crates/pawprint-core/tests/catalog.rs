//! Whole-catalog checks on the bundled mascots: alias audit (SPEC Appendix
//! A), night actions (SPEC §12.5), and the PNG export round trip (§12.7).

use pawprint_core::catalog::{Catalog, library};
use pawprint_core::matching::{TokenMatch, squash, token_match, whole_name};
use pawprint_core::sprite::{Origin, When, compile, export_png};

/// Words that show up in hostnames on their own. An alias equal to one of
/// these would pull every `web-01`, `lab-3`, or `bug-tracker` onto the same
/// mascot (rules 3 and 4), which defeats telling machines apart.
const HOSTNAME_WORDS: &[&str] = &[
    // roles and services
    "db",
    "database",
    "sql",
    "mysql",
    "postgres",
    "pg",
    "redis",
    "mongo",
    "web",
    "www",
    "http",
    "https",
    "nginx",
    "apache",
    "app",
    "apps",
    "api",
    "svc",
    "service",
    "auth",
    "sso",
    "ldap",
    "mail",
    "smtp",
    "imap",
    "mx",
    "ns",
    "dns",
    "dhcp",
    "ntp",
    "vpn",
    "proxy",
    "lb",
    "gw",
    "gateway",
    "router",
    "fw",
    "firewall",
    "switch",
    "ap",
    "wifi",
    "git",
    "repo",
    "registry",
    "ci",
    "cd",
    "build",
    "builder",
    "runner",
    "agent",
    "worker",
    "jenkins",
    "bot",
    "cron",
    "queue",
    "mq",
    "kafka",
    "cache",
    "cdn",
    "edge",
    "data",
    "files",
    "file",
    "share",
    "storage",
    "nas",
    "san",
    "disk",
    "backup",
    "bak",
    "archive",
    "log",
    "logs",
    "monitor",
    "monitoring",
    "mon",
    "metrics",
    "grafana",
    "prometheus",
    "search",
    "index",
    "wiki",
    "docs",
    "jira",
    "bug",
    "bugs",
    "tracker",
    "chat",
    "media",
    "plex",
    "print",
    "printer",
    "scan",
    "game",
    "games",
    "minecraft",
    "ml",
    "gpu",
    "ai",
    "compute",
    "hpc",
    // environments
    "prod",
    "production",
    "prd",
    "stage",
    "staging",
    "stg",
    "dev",
    "devel",
    "develop",
    "test",
    "testing",
    "tst",
    "qa",
    "uat",
    "demo",
    "sandbox",
    "lab",
    "labs",
    "preview",
    "canary",
    "beta",
    "alpha",
    "local",
    "localhost",
    "internal",
    "public",
    "private",
    "dmz",
    // machines
    "srv",
    "server",
    "node",
    "host",
    "box",
    "vm",
    "vps",
    "kvm",
    "esxi",
    "hyperv",
    "cloud",
    "cluster",
    "k",
    "kube",
    "master",
    "primary",
    "replica",
    "secondary",
    "standby",
    "mini",
    "macmini",
    "pro",
    "mbp",
    "macbook",
    "air",
    "imac",
    "studio",
    "laptop",
    "desktop",
    "pc",
    "workstation",
    "ws",
    "tower",
    "nuc",
    "pi",
    "rpi",
    "raspberrypi",
    "jetson",
    "mac",
    "win",
    "windows",
    "linux",
    "ubuntu",
    "debian",
    "fedora",
    "arch",
    "centos",
    "rhel",
    "alpine",
    "nixos",
    "bsd",
    "phone",
    "tablet",
    "tv",
    "console",
    "kiosk",
    "term",
    "shell",
    "bastion",
    "jump",
    "jumpbox",
    "admin",
    "mgmt",
    "ops",
    "infra",
    "core",
    "main",
    "home",
    "office",
    "work",
    "shop",
    "garage",
    "den",
    "attic",
    "basement",
    "kitchen",
    // shapes of names: colors, ordinals, places
    "red",
    "blue",
    "green",
    "black",
    "white",
    "grey",
    "gray",
    "orange",
    "yellow",
    "purple",
    "gold",
    "silver",
    "one",
    "two",
    "three",
    "first",
    "new",
    "old",
    "big",
    "small",
    "east",
    "west",
    "north",
    "south",
    "us",
    "eu",
    "ap",
    "az",
    "zone",
    "rack",
    "site",
    "dc",
];

#[test]
fn no_alias_is_a_common_hostname_word() {
    let offenders: Vec<String> = Catalog::bundled()
        .mascots()
        .iter()
        .flat_map(|m| m.aliases.iter().map(move |a| (m, squash(a))))
        .filter(|(_, a)| HOSTNAME_WORDS.contains(&a.as_str()))
        .map(|(m, a)| format!("{a} ({})", m.id))
        .collect();
    assert!(
        offenders.is_empty(),
        "aliases that are common hostname words: {offenders:?}"
    );
}

/// Realistic machine names with no animal in them. None may match an
/// alias by rule 3 or 4: each goes to the hash, so a fleet spreads out.
const NO_ANIMAL: &[&str] = &[
    "web-01",
    "web02",
    "db-prod-3",
    "db-replica-2",
    "build-mini",
    "build-agent-7",
    "dev-laptop",
    "lab-node-04",
    "lab3",
    "bug-tracker",
    "monitor-01",
    "grafana-mon",
    "disk-array-2",
    "nas",
    "nas-01",
    "backup-nas",
    "grey-box",
    "bot-runner-3",
    "ci-bot",
    "k8s-node-12",
    "k8s-master-1",
    "edge-gw-2",
    "proxy-eu-1",
    "mail",
    "smtp-out-2",
    "git",
    "gitlab-runner-4",
    "jenkins-agent-9",
    "redis-cache-1",
    "kafka-3",
    "mbp-16",
    "macbook-pro",
    "mac-mini-m2",
    "imac-27",
    "studio",
    "nuc-living",
    "raspberrypi",
    "pi4-kitchen",
    "ubuntu-2404",
    "win11-vm",
    "esxi-03",
    "hyperv-host-2",
    "prod-api-07",
    "stg-web-2",
    "qa-runner",
    "uat-app-1",
    "sandbox-12",
    "gpu-box",
    "ml-train-4",
    "plex",
    "media-server",
    "printer-office",
    "home-assistant",
    "router",
    "fw-01",
    "vpn-gw",
    "dns1",
    "ntp-2",
    "ldap-primary",
    "sso",
    "jump-01",
    "bastion",
    "admin-ws-3",
    "ops-console",
    "infra-core-1",
    "dc1-rack4-u12",
    "us-east-1a",
    "eu-west-node",
    "blue-01",
    "green-02",
    "red-team-3",
    "black-box",
    "white-tower",
    "gold-master",
    "silver-2",
    "alpha",
    "beta-3",
    "canary-1",
    "workstation",
    "desktop-7h2k9",
    "laptop-jane",
    "pc-1234",
    "tower-2",
    "minecraft",
    "terminal-server",
    "tv-room",
    "console-2",
    "kiosk-lobby",
    "shell-1",
    "build-mac-arm64",
];

#[test]
fn ordinary_hostnames_match_no_alias() {
    let aliases = Catalog::bundled().aliases();
    let surprises: Vec<String> = NO_ANIMAL
        .iter()
        .filter_map(|host| {
            let hit = whole_name(host, aliases)
                .map(|(name, id)| format!("{name} → {id}"))
                .or(match token_match(host, aliases) {
                    TokenMatch::One { ngram, id } => Some(format!("{ngram} → {id}")),
                    TokenMatch::Ambiguous(hits) => Some(format!("{hits:?}")),
                    TokenMatch::None => None,
                });
            hit.map(|h| format!("{host}: {h}"))
        })
        .collect();
    assert!(
        surprises.is_empty(),
        "ordinary hostnames that match a mascot: {surprises:#?}"
    );
}

/// Names with an animal in them get that animal.
#[test]
fn animal_hostnames_match_their_mascot() {
    let aliases = Catalog::bundled().aliases();
    let cases = [
        ("owl", "owl"),
        ("prod-owl-02", "owl"),
        ("polar-bear-3", "polar-bear"),
        ("bear-den", "bear"),
        ("red-panda", "red-panda"),
        ("dog-walker", "dog-labrador"),
        ("labrador-01", "dog-labrador"),
        ("beetle-1", "beetle"),
        ("floppy", "floppy"),
        ("crt-lab", "crt"),
        ("robot-arm-2", "robot"),
        ("greyhound", "dog-greyhound"),
        ("graycat", "cat-gray"),
        ("buildbox-otter", "otter"),
        ("tux", "cat-tuxedo"),
    ];
    for (host, want) in cases {
        let got =
            whole_name(host, aliases)
                .map(|(_, id)| id)
                .or(match token_match(host, aliases) {
                    TokenMatch::One { id, .. } => Some(id),
                    _ => None,
                });
        assert_eq!(got.as_deref(), Some(want), "{host}");
    }
}

/// SPEC §12.5: the nocturnal set, and each has something to do at night.
#[test]
fn nocturnal_mascots_have_night_actions() {
    let nocturnal: Vec<&str> = Catalog::bundled()
        .mascots()
        .iter()
        .filter(|m| m.nocturnal)
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(
        nocturnal,
        ["cat-black", "ghost", "moth", "opossum", "owl", "raccoon"]
    );
    for id in nocturnal {
        let m = Catalog::bundled().get(id).unwrap();
        assert!(
            m.actions.iter().any(|a| a.when == When::Night),
            "{id} is nocturnal but has no `when = \"night\"` action"
        );
    }
}

/// `pawprint mascot export` output is a valid user mascot that looks the
/// same as the original, for every bundled mascot.
#[test]
fn every_bundled_mascot_exports_to_a_valid_png_mascot() {
    for m in Catalog::bundled().mascots() {
        let (toml, strip) = export_png(m);
        let png = strip.encode_png();
        let c = compile(&toml, Some(&m.id), library(), Origin::User, &|_| {
            Ok(png.clone())
        });
        assert!(c.report.is_clean(), "{}: {:?}", m.id, c.report);
        let back = c.mascot.unwrap();
        assert_eq!(back.frames.len(), m.frames.len(), "{}", m.id);
        for i in 0..m.frames.len() {
            assert_eq!(back.frame_rgba(i), m.frame_rgba(i), "{} frame {i}", m.id);
        }
        assert_eq!(back.actions.len(), m.actions.len(), "{}", m.id);
        for (a, b) in m.actions.iter().zip(&back.actions) {
            // Same frames in the same order once holds are merged, and
            // timing within 50 ms per frame.
            let merged = |f: &[pawprint_core::sprite::FrameRef]| {
                let mut out: Vec<(u16, u32)> = Vec::new();
                for r in f {
                    match out.last_mut() {
                        Some((frame, ms)) if *frame == r.frame => *ms += u32::from(r.ms),
                        _ => out.push((r.frame, u32::from(r.ms))),
                    }
                }
                out
            };
            let (ma, mb) = (merged(&a.frames), merged(&b.frames));
            let ctx = format!("{} action {}", m.id, a.name);
            assert_eq!(
                ma.iter().map(|x| x.0).collect::<Vec<_>>(),
                mb.iter().map(|x| x.0).collect::<Vec<_>>(),
                "{ctx}"
            );
            for ((_, x), (_, y)) in ma.iter().zip(&mb) {
                assert!(x.abs_diff(*y) <= 50, "{ctx}: {x} ms became {y} ms");
            }
        }
    }
}
