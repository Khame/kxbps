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
//! `ftp.debian.ru`). They are kept, in place, because the web app keeps them and
//! shows the resulting error in the UI: [`crate::probe::check_probe_url`] refuses
//! plain HTTP before a request is made, so those rows always end as
//! `ProbeError::PlainHttp` and never report a latency. They are the live case the
//! Mirrors view has to render; the other 32 entries are `https://`.
//!
//! The web app's `tier` (1 or 2) is carried: `xmirror`, the official mirror
//! selector, prints it next to every row, and the view shows it as a column.
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
    /// Tier from `mirrors.yaml`: `1` is run by the Void infrastructure team and
    /// always current, `2` is a community mirror with no freshness guarantee.
    ///
    /// `repo-default` has no entry of its own upstream; it redirects to a Tier 1
    /// mirror, so it carries `1`.
    pub tier: u8,
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
        tier: 1,
    },
    MirrorInfo {
        name: "Helsinki",
        url: "https://repo-fi.voidlinux.org/",
        region: Region::Eu,
        country: "Finland",
        tier: 1,
    },
    MirrorInfo {
        name: "Frankfurt",
        url: "https://repo-de.voidlinux.org/",
        region: Region::Eu,
        country: "Germany",
        tier: 1,
    },
    MirrorInfo {
        name: "Fastly Global CDN",
        url: "https://repo-fastly.voidlinux.org/",
        region: Region::World,
        country: "",
        tier: 1,
    },
    MirrorInfo {
        name: "Chicago",
        url: "https://mirrors.summithq.com/voidlinux/",
        region: Region::Na,
        country: "USA",
        tier: 1,
    },
    MirrorInfo {
        name: "Cloudflare Global CDN",
        url: "https://mirrors.cicku.me/voidlinux/",
        region: Region::World,
        country: "",
        tier: 2,
    },
    MirrorInfo {
        name: "China",
        url: "https://mirror.nju.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
        tier: 2,
    },
    MirrorInfo {
        name: "Beijing",
        url: "https://mirrors.bfsu.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
        tier: 2,
    },
    MirrorInfo {
        name: "Beijing",
        url: "https://mirrors.tuna.tsinghua.edu.cn/voidlinux/",
        region: Region::As,
        country: "China",
        tier: 2,
    },
    MirrorInfo {
        name: "Tehran",
        url: "https://mirror.mmdbalkhi.ir/void/",
        region: Region::As,
        country: "Iran",
        tier: 2,
    },
    MirrorInfo {
        name: "Tehran",
        url: "https://mirrors.kubarcloud.net/voidlinux/",
        region: Region::As,
        country: "Iran",
        tier: 2,
    },
    MirrorInfo {
        name: "Tokyo",
        url: "https://repo.jing.rocks/voidlinux/",
        region: Region::As,
        country: "Japan",
        tier: 2,
    },
    MirrorInfo {
        name: "Singapore",
        url: "https://mirror.freedif.org/voidlinux/",
        region: Region::As,
        country: "Singapore",
        tier: 2,
    },
    MirrorInfo {
        name: "South Korea",
        url: "https://mirror.heigou.pe.kr/voidlinux/",
        region: Region::As,
        country: "South Korea",
        tier: 2,
    },
    MirrorInfo {
        name: "Hanoi",
        url: "https://mirror.meowsmp.net/voidlinux/",
        region: Region::As,
        country: "Vietnam",
        tier: 2,
    },
    MirrorInfo {
        name: "Belgium",
        url: "https://voidlinux.mirror-services.net/voidlinux/",
        region: Region::Eu,
        country: "Belgium",
        tier: 2,
    },
    MirrorInfo {
        name: "Denmark",
        url: "http://ftp.dk.xemacs.org/voidlinux/",
        region: Region::Eu,
        country: "Denmark",
        tier: 2,
    },
    MirrorInfo {
        name: "Denmark",
        url: "https://mirrors.dotsrc.org/voidlinux/",
        region: Region::Eu,
        country: "Denmark",
        tier: 2,
    },
    MirrorInfo {
        name: "Nuremberg",
        url: "https://void.mirror.lynnfra.net/",
        region: Region::Eu,
        country: "Germany",
        tier: 2,
    },
    MirrorInfo {
        name: "Greece",
        url: "https://ftp.cc.uoc.gr/mirrors/linux/voidlinux/",
        region: Region::Eu,
        country: "Greece",
        tier: 2,
    },
    MirrorInfo {
        name: "Italy",
        url: "https://voidlinux.mirror.garr.it/",
        region: Region::Eu,
        country: "Italy",
        tier: 2,
    },
    MirrorInfo {
        name: "Warsaw",
        url: "https://void.sakamoto.pl/",
        region: Region::Eu,
        country: "Poland",
        tier: 2,
    },
    MirrorInfo {
        name: "Russia",
        url: "http://ftp.debian.ru/mirrors/voidlinux/",
        region: Region::Eu,
        country: "Russia",
        tier: 2,
    },
    MirrorInfo {
        name: "Russia",
        url: "https://mirror.yandex.ru/mirrors/voidlinux/",
        region: Region::Eu,
        country: "Russia",
        tier: 2,
    },
    MirrorInfo {
        name: "Sweden",
        url: "https://ftp.lysator.liu.se/pub/voidlinux/",
        region: Region::Eu,
        country: "Sweden",
        tier: 2,
    },
    MirrorInfo {
        name: "Sweden",
        url: "https://mirror.accum.se/mirror/voidlinux/",
        region: Region::Eu,
        country: "Sweden",
        tier: 2,
    },
    MirrorInfo {
        name: "Bern",
        url: "https://mirror.puzzle.ch/voidlinux/",
        region: Region::Eu,
        country: "Switzerland",
        tier: 2,
    },
    MirrorInfo {
        name: "Virginia",
        url: "https://mirror.vofr.net/voidlinux/",
        region: Region::Na,
        country: "USA",
        tier: 2,
    },
    MirrorInfo {
        name: "Michigan",
        url: "https://mirrors.lug.mtu.edu/voidlinux/",
        region: Region::Na,
        country: "USA",
        tier: 2,
    },
    MirrorInfo {
        name: "Montreal",
        url: "https://void.tuxmirrors.xyz/",
        region: Region::Na,
        country: "Canada",
        tier: 2,
    },
    MirrorInfo {
        name: "Canberra",
        url: "https://mirror.aarnet.edu.au/pub/voidlinux/",
        region: Region::Oc,
        country: "Australia",
        tier: 2,
    },
    MirrorInfo {
        name: "Mirante do Paranapanema",
        url: "https://void.voidbr.org/voidlinux/",
        region: Region::Sa,
        country: "Brazil",
        tier: 2,
    },
    MirrorInfo {
        name: "Cacoal",
        url: "https://void.voidlinux.com.br/voidlinux/",
        region: Region::Sa,
        country: "Brazil",
        tier: 2,
    },
    MirrorInfo {
        name: "Quito",
        url: "https://mirror.linux.ec/voidlinux/",
        region: Region::Sa,
        country: "Ecuador",
        tier: 2,
    },
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::probe::{ProbeError, check_probe_url};

    /// The mirror list, transcribed independently: one mirror per line as
    /// `name|url|region|country|tier`, same order. Any edit to [`MIRRORS`] that
    /// drifts from `mirrors.ts` (and from `mirrors.yaml`'s tier column) fails
    /// here.
    const WEB_LIST: &str = "\
Default redirect|https://repo-default.voidlinux.org/|World||1
Helsinki|https://repo-fi.voidlinux.org/|EU|Finland|1
Frankfurt|https://repo-de.voidlinux.org/|EU|Germany|1
Fastly Global CDN|https://repo-fastly.voidlinux.org/|World||1
Chicago|https://mirrors.summithq.com/voidlinux/|NA|USA|1
Cloudflare Global CDN|https://mirrors.cicku.me/voidlinux/|World||2
China|https://mirror.nju.edu.cn/voidlinux/|AS|China|2
Beijing|https://mirrors.bfsu.edu.cn/voidlinux/|AS|China|2
Beijing|https://mirrors.tuna.tsinghua.edu.cn/voidlinux/|AS|China|2
Tehran|https://mirror.mmdbalkhi.ir/void/|AS|Iran|2
Tehran|https://mirrors.kubarcloud.net/voidlinux/|AS|Iran|2
Tokyo|https://repo.jing.rocks/voidlinux/|AS|Japan|2
Singapore|https://mirror.freedif.org/voidlinux/|AS|Singapore|2
South Korea|https://mirror.heigou.pe.kr/voidlinux/|AS|South Korea|2
Hanoi|https://mirror.meowsmp.net/voidlinux/|AS|Vietnam|2
Belgium|https://voidlinux.mirror-services.net/voidlinux/|EU|Belgium|2
Denmark|http://ftp.dk.xemacs.org/voidlinux/|EU|Denmark|2
Denmark|https://mirrors.dotsrc.org/voidlinux/|EU|Denmark|2
Nuremberg|https://void.mirror.lynnfra.net/|EU|Germany|2
Greece|https://ftp.cc.uoc.gr/mirrors/linux/voidlinux/|EU|Greece|2
Italy|https://voidlinux.mirror.garr.it/|EU|Italy|2
Warsaw|https://void.sakamoto.pl/|EU|Poland|2
Russia|http://ftp.debian.ru/mirrors/voidlinux/|EU|Russia|2
Russia|https://mirror.yandex.ru/mirrors/voidlinux/|EU|Russia|2
Sweden|https://ftp.lysator.liu.se/pub/voidlinux/|EU|Sweden|2
Sweden|https://mirror.accum.se/mirror/voidlinux/|EU|Sweden|2
Bern|https://mirror.puzzle.ch/voidlinux/|EU|Switzerland|2
Virginia|https://mirror.vofr.net/voidlinux/|NA|USA|2
Michigan|https://mirrors.lug.mtu.edu/voidlinux/|NA|USA|2
Montreal|https://void.tuxmirrors.xyz/|NA|Canada|2
Canberra|https://mirror.aarnet.edu.au/pub/voidlinux/|OC|Australia|2
Mirante do Paranapanema|https://void.voidbr.org/voidlinux/|SA|Brazil|2
Cacoal|https://void.voidlinux.com.br/voidlinux/|SA|Brazil|2
Quito|https://mirror.linux.ec/voidlinux/|SA|Ecuador|2";

    /// Splits [`WEB_LIST`] into `(name, url, region, country, tier)` rows.
    fn web_list() -> Vec<(&'static str, &'static str, &'static str, &'static str, u8)> {
        WEB_LIST
            .lines()
            .map(|line| {
                let mut fields = line.split('|');
                let row = (
                    fields.next().expect("name"),
                    fields.next().expect("url"),
                    fields.next().expect("region"),
                    fields.next().expect("country"),
                    fields
                        .next()
                        .expect("tier")
                        .parse()
                        .expect("tier is a number"),
                );
                assert!(fields.next().is_none(), "extra field in {line:?}");
                row
            })
            .collect()
    }

    #[test]
    fn list_is_not_empty() {
        assert!(!MIRRORS.is_empty());
        assert_eq!(MIRRORS.len(), 34);
        assert_eq!(MIRRORS.len(), web_list().len());
    }

    #[test]
    fn list_matches_the_web_app() {
        let actual: Vec<(&str, &str, &str, &str, u8)> = MIRRORS
            .iter()
            .map(|m| (m.name, m.url, m.region.as_str(), m.country, m.tier))
            .collect();
        assert_eq!(actual, web_list());
    }

    /// The tier column `xmirror` prints: four upstream mirrors and the default
    /// redirect are Tier 1, everything else is Tier 2.
    #[test]
    fn the_tier_column_matches_xmirror() {
        let tier_one: Vec<&str> = MIRRORS
            .iter()
            .filter(|m| m.tier == 1)
            .map(|m| m.url)
            .collect();
        assert_eq!(
            tier_one,
            [
                "https://repo-default.voidlinux.org/",
                "https://repo-fi.voidlinux.org/",
                "https://repo-de.voidlinux.org/",
                "https://repo-fastly.voidlinux.org/",
                "https://mirrors.summithq.com/voidlinux/",
            ]
        );
        assert!(MIRRORS.iter().all(|m| m.tier == 1 || m.tier == 2));
        assert_eq!(MIRRORS.iter().filter(|m| m.tier == 2).count(), 29);
    }

    #[test]
    fn default_mirror_is_the_web_app_default_and_the_first_row() {
        assert_eq!(DEFAULT_MIRROR, "https://repo-default.voidlinux.org/");
        assert_eq!(MIRRORS[0].url, DEFAULT_MIRROR);
        assert_eq!(MIRRORS[0].name, "Default redirect");
        assert_eq!(MIRRORS[0].region, Region::World);
    }

    #[test]
    fn every_url_uses_http_or_https_and_ends_with_a_slash() {
        for mirror in MIRRORS {
            assert!(
                mirror.url.starts_with("https://") || mirror.url.starts_with("http://"),
                "{} is not http(s)",
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
    fn plain_http_mirrors_are_rejected_by_the_probe_rule() {
        let plain: Vec<&str> = MIRRORS
            .iter()
            .map(|m| m.url)
            .filter(|url| url.starts_with("http://"))
            .collect();
        // The two entries the web app carries and the Mirrors view must show as
        // a plain-HTTP error.
        assert_eq!(
            plain,
            [
                "http://ftp.dk.xemacs.org/voidlinux/",
                "http://ftp.debian.ru/mirrors/voidlinux/",
            ]
        );
        for url in plain {
            assert_eq!(check_probe_url(url), Err(ProbeError::PlainHttp), "{url}");
        }
    }

    #[test]
    fn probe_rule_verdict_matches_the_scheme_of_every_mirror() {
        for mirror in MIRRORS {
            let expected = if mirror.url.starts_with("http://") {
                Err(ProbeError::PlainHttp)
            } else {
                Ok(())
            };
            assert_eq!(check_probe_url(mirror.url), expected, "{}", mirror.url);
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
                "tier": 1,
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
