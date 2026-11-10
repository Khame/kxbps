//! The mirror list the user picks from, and its regions.
//!
//! Ported from the web app's `mirrors.ts` (`MIRRORS`, `DEFAULT_MIRROR`,
//! `MirrorRegion`), which in turn comes from void-linux/xmirror's `mirrors.yaml`
//! plus `repo-default`. The list is static data: it is compiled in, never
//! fetched, and it keeps the web app's order, so the Mirrors view shows the same
//! rows in the same sequence.
//!
//! Two fields are derived, because the web app stores a single `location`
//! string while the Qt model needs a label and a country:
//!
//! - [`MirrorInfo::name`] is the place label — `location` without its trailing
//!   country (`"Helsinki, Finland"` → `"Helsinki"`). When the location is only a
//!   country (`"China"`) the name is that country, and the CDN entries and the
//!   default redirect keep their location verbatim.
//! - [`MirrorInfo::country`] is the country (`"Finland"`, `"China"`), or empty
//!   for the global CDNs and the default redirect, which serve no single
//!   country. A view should render it only when it differs from the name.
//!
//! Two entries of the web app's list are `http://` (`ftp.dk.xemacs.org`,
//! `ftp.debian.ru`). They are not ported: [`crate::probe::check_probe_url`]
//! refuses plain HTTP before a request is made, exactly as the web app does, so
//! those two could never report a latency — they would sit in the list as
//! permanent failures. Everything in [`MIRRORS`] is `https://`.
//!
//! The web app's `tier` (1 or 2) is not carried: the native view has no tier
//! column, and a field nothing reads would only invite drift.
//!
//! [`MirrorInfo`] derives `Serialize` but not `Deserialize`: the list is
//! compiled in and never parsed, and its `&'static str` fields could not be
//! deserialised without turning them into owned strings.

use serde::Serialize;

/// Region a mirror serves (`MirrorRegion` in the web app).
///
/// Serialises as the web app's tag (`"EU"`, `"World"`, …) so the value the QML
/// side sees is the one the web app uses for its filter chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Region {
    #[serde(rename = "EU")]
    Eu,
    #[serde(rename = "NA")]
    Na,
    #[serde(rename = "AS")]
    As,
    #[serde(rename = "OC")]
    Oc,
    #[serde(rename = "SA")]
    Sa,
    #[serde(rename = "World")]
    World,
}

impl Region {
    /// Every region, in the web app's filter-chip order.
    pub const ALL: [Region; 6] = [
        Region::Eu,
        Region::Na,
        Region::As,
        Region::Oc,
        Region::Sa,
        Region::World,
    ];

    /// The web app's tag for this region.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eu => "EU",
            Self::Na => "NA",
            Self::As => "AS",
            Self::Oc => "OC",
            Self::Sa => "SA",
            Self::World => "World",
        }
    }
}

/// One selectable mirror (`Mirror` in the web app's `mirrors.ts`).
///
/// Serialises to `{ "name": …, "url": …, "region": …, "country": … }`, the
/// shape the Qt mirror model exposes as roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MirrorInfo {
    /// Place label, see the module documentation.
    pub name: &'static str,
    /// Repository base URL, always `https://` and always ending in `/` so
    /// `current/` can be appended directly.
    pub url: &'static str,
    pub region: Region,
    /// Country, empty when the mirror serves no single country.
    pub country: &'static str,
}

/// Mirror selected until the user picks another one (`DEFAULT_MIRROR` in the web
/// app, and the first row of [`MIRRORS`]).
pub const DEFAULT_MIRROR: &str = "https://repo-default.voidlinux.org/";

/// Every mirror, in the web app's order.
pub const MIRRORS: &[MirrorInfo] = &[
    MirrorInfo {
        name: "Default redirect",
        url: DEFAULT_MIRROR,
        region: Region::World,
        country: "",
    },
    MirrorInfo {
        name: "Helsinki",
        url: "https://repo-fi.voidlinux.org/",
        region: Region::Eu,
        country: "Finland",
    },
    MirrorInfo {
        name: "Frankfurt",
        url: "https://repo-de.voidlinux.org/",
        region: Region::Eu,
        country: "Germany",
    },
    MirrorInfo {
        name: "Fastly Global CDN",
        url: "https://repo-fastly.voidlinux.org/",
        region: Region::World,
        country: "",
    },
    MirrorInfo {
        name: "Chicago",
        url: "https://mirrors.summithq.com/voidlinux/",
        region: Region::Na,
        country: "USA",
    },
    MirrorInfo {
        name: "Cloudflare Global CDN",
        url: "https://mirrors.cicku.me/voidlinux/",
        region: Region::World,
        country: "",
    },
    MirrorInfo {
        name: "China",
        url: "https://mirror.nju.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
    },
    MirrorInfo {
        name: "Beijing",
        url: "https://mirrors.bfsu.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
    },
    MirrorInfo {
        name: "Beijing",
        url: "https://mirrors.tuna.tsinghua.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
    },
    MirrorInfo {
        name: "Tehran",
        url: "https://mirror.mmdbalkhi.ir/void/",
        region: Region::As,
        country: "Iran",
    },
    MirrorInfo {
        name: "Tehran",
        url: "https://mirrors.kubarcloud.net/voidlinux/",
        region: Region::As,
        country: "Iran",
    },
    MirrorInfo {
        name: "Tokyo",
        url: "https://repo.jing.rocks/voidlinux/",
        region: Region::As,
        country: "Japan",
    },
    MirrorInfo {
        name: "Singapore",
        url: "https://mirror.freedif.org/voidlinux/",
        region: Region::As,
        country: "Singapore",
    },
    MirrorInfo {
        name: "South Korea",
        url: "https://mirror.heigou.pe.kr/voidlinux/",
        region: Region::As,
        country: "South Korea",
    },
    MirrorInfo {
        name: "Hanoi",
        url: "https://mirror.meowsmp.net/voidlinux/",
        region: Region::As,
        country: "Vietnam",
    },
    MirrorInfo {
        name: "Belgium",
        url: "https://voidlinux.mirror-services.net/voidlinux/",
        region: Region::Eu,
        country: "Belgium",
    },
    MirrorInfo {
        name: "Denmark",
        url: "https://mirrors.dotsrc.org/voidlinux/",
        region: Region::Eu,
        country: "Denmark",
    },
    MirrorInfo {
        name: "Nuremberg",
        url: "https://void.mirror.lynnfra.net/",
        region: Region::Eu,
        country: "Germany",
    },
    MirrorInfo {
        name: "Greece",
        url: "https://ftp.cc.uoc.gr/mirrors/linux/voidlinux/",
        region: Region::Eu,
        country: "Greece",
    },
    MirrorInfo {
        name: "Italy",
        url: "https://voidlinux.mirror.garr.it/",
        region: Region::Eu,
        country: "Italy",
    },
    MirrorInfo {
        name: "Warsaw",
        url: "https://void.sakamoto.pl/",
        region: Region::Eu,
        country: "Poland",
    },
    MirrorInfo {
        name: "Russia",
        url: "https://mirror.yandex.ru/mirrors/voidlinux/",
        region: Region::Eu,
        country: "Russia",
    },
    MirrorInfo {
        name: "Sweden",
        url: "https://ftp.lysator.liu.se/pub/voidlinux/",
        region: Region::Eu,
        country: "Sweden",
    },
    MirrorInfo {
        name: "Sweden",
        url: "https://mirror.accum.se/mirror/voidlinux/",
        region: Region::Eu,
        country: "Sweden",
    },
    MirrorInfo {
        name: "Bern",
        url: "https://mirror.puzzle.ch/voidlinux/",
        region: Region::Eu,
        country: "Switzerland",
    },
    MirrorInfo {
        name: "Virginia",
        url: "https://mirror.vofr.net/voidlinux/",
        region: Region::Na,
        country: "USA",
    },
    MirrorInfo {
        name: "Michigan",
        url: "https://mirrors.lug.mtu.edu/voidlinux/",
        region: Region::Na,
        country: "USA",
    },
    MirrorInfo {
        name: "Montreal",
        url: "https://void.tuxmirrors.xyz/",
        region: Region::Na,
        country: "Canada",
    },
    MirrorInfo {
        name: "Canberra",
        url: "https://mirror.aarnet.edu.au/pub/voidlinux/",
        region: Region::Oc,
        country: "Australia",
    },
    MirrorInfo {
        name: "Mirante do Paranapanema",
        url: "https://void.voidbr.org/voidlinux/",
        region: Region::Sa,
        country: "Brazil",
    },
    MirrorInfo {
        name: "Cacoal",
        url: "https://void.voidlinux.com.br/voidlinux/",
        region: Region::Sa,
        country: "Brazil",
    },
    MirrorInfo {
        name: "Quito",
        url: "https://mirror.linux.ec/voidlinux/",
        region: Region::Sa,
        country: "Ecuador",
    },
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::probe::check_probe_url;

    /// The web app's list, transcribed independently: one mirror per line as
    /// `name|url|region|country`, same order, with the two `http://` entries
    /// left out (see the module documentation). Any edit to [`MIRRORS`] that
    /// drifts from `mirrors.ts` fails here.
    const WEB_LIST: &str = "\
Default redirect|https://repo-default.voidlinux.org/|World|
Helsinki|https://repo-fi.voidlinux.org/|EU|Finland
Frankfurt|https://repo-de.voidlinux.org/|EU|Germany
Fastly Global CDN|https://repo-fastly.voidlinux.org/|World|
Chicago|https://mirrors.summithq.com/voidlinux/|NA|USA
Cloudflare Global CDN|https://mirrors.cicku.me/voidlinux/|World|
China|https://mirror.nju.edu.cn/voidlinux/|AS|China
Beijing|https://mirrors.bfsu.edu.cn/voidlinux/|AS|China
Beijing|https://mirrors.tuna.tsinghua.edu.cn/voidlinux/|AS|China
Tehran|https://mirror.mmdbalkhi.ir/void/|AS|Iran
Tehran|https://mirrors.kubarcloud.net/voidlinux/|AS|Iran
Tokyo|https://repo.jing.rocks/voidlinux/|AS|Japan
Singapore|https://mirror.freedif.org/voidlinux/|AS|Singapore
South Korea|https://mirror.heigou.pe.kr/voidlinux/|AS|South Korea
Hanoi|https://mirror.meowsmp.net/voidlinux/|AS|Vietnam
Belgium|https://voidlinux.mirror-services.net/voidlinux/|EU|Belgium
Denmark|https://mirrors.dotsrc.org/voidlinux/|EU|Denmark
Nuremberg|https://void.mirror.lynnfra.net/|EU|Germany
Greece|https://ftp.cc.uoc.gr/mirrors/linux/voidlinux/|EU|Greece
Italy|https://voidlinux.mirror.garr.it/|EU|Italy
Warsaw|https://void.sakamoto.pl/|EU|Poland
Russia|https://mirror.yandex.ru/mirrors/voidlinux/|EU|Russia
Sweden|https://ftp.lysator.liu.se/pub/voidlinux/|EU|Sweden
Sweden|https://mirror.accum.se/mirror/voidlinux/|EU|Sweden
Bern|https://mirror.puzzle.ch/voidlinux/|EU|Switzerland
Virginia|https://mirror.vofr.net/voidlinux/|NA|USA
Michigan|https://mirrors.lug.mtu.edu/voidlinux/|NA|USA
Montreal|https://void.tuxmirrors.xyz/|NA|Canada
Canberra|https://mirror.aarnet.edu.au/pub/voidlinux/|OC|Australia
Mirante do Paranapanema|https://void.voidbr.org/voidlinux/|SA|Brazil
Cacoal|https://void.voidlinux.com.br/voidlinux/|SA|Brazil
Quito|https://mirror.linux.ec/voidlinux/|SA|Ecuador
";

    /// Splits [`WEB_LIST`] into `(name, url, region, country)` rows.
    fn web_list() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
        WEB_LIST
            .lines()
            .map(|line| {
                let mut fields = line.split('|');
                let row = (
                    fields.next().expect("name"),
                    fields.next().expect("url"),
                    fields.next().expect("region"),
                    fields.next().expect("country"),
                );
                assert!(fields.next().is_none(), "extra field in {line:?}");
                row
            })
            .collect()
    }

    #[test]
    fn list_is_not_empty() {
        assert!(!MIRRORS.is_empty());
        assert_eq!(MIRRORS.len(), 32);
        assert_eq!(MIRRORS.len(), web_list().len());
    }

    #[test]
    fn list_matches_the_web_app() {
        let actual: Vec<(&str, &str, &str, &str)> = MIRRORS
            .iter()
            .map(|m| (m.name, m.url, m.region.as_str(), m.country))
            .collect();
        assert_eq!(actual, web_list());
    }

    #[test]
    fn default_mirror_is_the_web_app_default_and_the_first_row() {
        assert_eq!(DEFAULT_MIRROR, "https://repo-default.voidlinux.org/");
        assert_eq!(MIRRORS[0].url, DEFAULT_MIRROR);
        assert_eq!(MIRRORS[0].name, "Default redirect");
        assert_eq!(MIRRORS[0].region, Region::World);
    }

    #[test]
    fn every_url_is_https_and_ends_with_a_slash() {
        for mirror in MIRRORS {
            assert!(
                mirror.url.starts_with("https://"),
                "{} is not https",
                mirror.url
            );
            assert!(
                mirror.url.ends_with('/'),
                "{} needs a trailing slash",
                mirror.url
            );
        }
    }

    #[test]
    fn every_url_passes_the_probe_rules() {
        for mirror in MIRRORS {
            assert_eq!(check_probe_url(mirror.url), Ok(()), "{}", mirror.url);
        }
    }

    #[test]
    fn urls_are_unique() {
        let unique: HashSet<&str> = MIRRORS.iter().map(|m| m.url).collect();
        assert_eq!(unique.len(), MIRRORS.len());
    }

    #[test]
    fn every_region_tag_round_trips_and_all_is_in_chip_order() {
        let tags: Vec<&str> = Region::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(tags, ["EU", "NA", "AS", "OC", "SA", "World"]);
        for mirror in MIRRORS {
            assert!(tags.contains(&mirror.region.as_str()));
        }
    }

    #[test]
    fn names_are_never_empty() {
        for mirror in MIRRORS {
            assert!(!mirror.name.is_empty(), "{} has no name", mirror.url);
        }
    }

    #[test]
    fn serialises_with_the_role_names_the_qt_model_exposes() {
        let json = serde_json::to_value(MIRRORS[1]).expect("serialise");
        assert_eq!(
            json,
            serde_json::json!({
                "name": "Helsinki",
                "url": "https://repo-fi.voidlinux.org/",
                "region": "EU",
                "country": "Finland",
            })
        );
    }

    #[test]
    fn every_region_is_covered_by_at_least_one_mirror() {
        for region in Region::ALL {
            assert!(
                MIRRORS.iter().any(|m| m.region == region),
                "no mirror in {}",
                region.as_str()
            );
        }
    }
}
