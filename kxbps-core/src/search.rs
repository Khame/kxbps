//! Ordering of a repository search answer.
//!
//! `xbps-query -Rs <pattern>` prints its hits in the repository index order,
//! which puts an exact name match wherever the alphabet leaves it: `vim` is
//! the eighteenth line of thirty for the pattern `vim`. The list a user reads
//! after typing a name should start with that name.
//!
//! Nothing here invents a relevance score. The three groups are the ones a user
//! can name — the package itself, the packages named after it, everything
//! else — and xbps' own order is kept inside each group, so the answer is a
//! reordering of the same lines and never a different set.

use crate::xbps::SearchHit;

/// Puts the exact name first, then the names that start with the pattern, then
/// the rest; xbps' order is kept inside each group.
///
/// The comparison is case-insensitive, like the list's own filter: typing
/// `Vim` must find `vim` and rank it the same way.
pub fn rank_search_hits(hits: &mut [SearchHit], pattern: &str) {
    let pattern = pattern.to_lowercase();
    if pattern.is_empty() {
        return;
    }
    // A stable sort by a three-valued key keeps the groups in order and leaves
    // the hits of one group in the order xbps printed them.
    hits.sort_by_key(|hit| group_of(hit, &pattern));
}

/// Which group a hit belongs to, as the sort key.
///
/// Lower is nearer the top: `0` is the exact name, `1` a name that starts with
/// the pattern, `2` everything else.
fn group_of(hit: &SearchHit, pattern: &str) -> u8 {
    let name = hit.name.to_lowercase();
    if name == pattern {
        0
    } else if name.starts_with(pattern) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One hit as `xbps-query -Rs` printed it: `[*]` means installed.
    fn hit(tag_installed: bool, name: &str, version: &str) -> SearchHit {
        SearchHit {
            installed: tag_installed,
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: format!("{name}-{version}"),
            description: format!("{name} description"),
        }
    }

    fn names(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.name.as_str()).collect()
    }

    /// The thirty lines `xbps-query -Rs vim` answered on this machine, in the
    /// order xbps printed them. `vim` itself is the eighteenth.
    fn vim_hits() -> Vec<SearchHit> {
        [
            "apvlv",
            "bob",
            "felix",
            "gvim",
            "gvim-huge",
            "iamb",
            "kakoune",
            "neovide",
            "neovim",
            "neovim-qt",
            "neovim-remote",
            "nvimpager",
            "python3-neovim",
            "python3-vint",
            "tree-sitter-vim",
            "tree-sitter-vimdoc",
            "vifm",
            "vim",
            "vim-colorschemes",
            "vim-common",
            "vim-gnupg",
            "vim-huge",
            "vim-huge-python3",
            "vim-x11",
            "vimb",
            "vimiv",
            "vimpager",
            "vimpc",
            "neovim-32bit",
            "vimb-32bit",
        ]
        .into_iter()
        .map(|name| hit(false, name, "1_1"))
        .collect()
    }

    #[test]
    fn the_exact_name_leads_a_real_repository_answer() {
        let mut hits = vim_hits();
        assert_eq!(hits.len(), 30);
        assert_eq!(hits[17].name, "vim", "xbps puts the exact match far down");
        rank_search_hits(&mut hits, "vim");
        assert_eq!(
            hits[0].name, "vim",
            "typing the package name must show that package first"
        );
        assert_eq!(hits.len(), 30, "ranking never drops or adds a hit");
    }

    #[test]
    fn the_groups_are_exact_then_prefix_then_the_rest() {
        let mut hits = vim_hits();
        rank_search_hits(&mut hits, "vim");
        // Everything that starts with "vim", in xbps' order, then the others.
        // `vimb-32bit` is the last line xbps printed and still a prefix hit, so
        // it closes the group: the sort is stable, not a second alphabetisation.
        let prefix_count = vim_hits()
            .iter()
            .filter(|hit| hit.name.starts_with("vim"))
            .count();
        assert_eq!(
            &names(&hits)[..prefix_count],
            &[
                "vim",
                "vim-colorschemes",
                "vim-common",
                "vim-gnupg",
                "vim-huge",
                "vim-huge-python3",
                "vim-x11",
                "vimb",
                "vimiv",
                "vimpager",
                "vimpc",
                "vimb-32bit",
            ],
        );
        assert!(
            hits[prefix_count..]
                .iter()
                .all(|hit| !hit.name.starts_with("vim")),
            "no prefix hit may follow a non-prefix one"
        );
    }

    #[test]
    fn a_hit_that_only_mentions_the_pattern_keeps_its_xbps_order() {
        // The tail is exactly the input order: this is a partition, not a
        // ranking of its own.
        let mut hits = vim_hits();
        let tail_before: Vec<String> = names(&hits)
            .into_iter()
            .filter(|name| !name.starts_with("vim"))
            .map(str::to_owned)
            .collect();
        rank_search_hits(&mut hits, "vim");
        let tail_after: Vec<String> = names(&hits)
            .into_iter()
            .filter(|name| !name.starts_with("vim"))
            .map(str::to_owned)
            .collect();
        assert_eq!(tail_before, tail_after);
    }

    #[test]
    fn the_pattern_is_matched_case_insensitively() {
        let mut hits = vec![
            hit(false, "vimb", "1_1"),
            hit(false, "VIM", "1_1"),
            hit(false, "apvlv", "1_1"),
        ];
        rank_search_hits(&mut hits, "Vim");
        assert_eq!(names(&hits), vec!["VIM", "vimb", "apvlv"]);
    }

    #[test]
    fn a_prefix_match_beats_a_hit_that_only_contains_the_pattern() {
        let mut hits = vec![hit(false, "apvlv", "1_1"), hit(false, "vimb", "1_1")];
        rank_search_hits(&mut hits, "vim");
        assert_eq!(names(&hits), vec!["vimb", "apvlv"]);
    }

    #[test]
    fn an_empty_pattern_leaves_the_answer_alone() {
        let mut hits = vim_hits();
        let before = names(&hits)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        rank_search_hits(&mut hits, "");
        let after = names(&hits)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(before, after);
    }

    #[test]
    fn an_installed_tag_does_not_move_a_hit() {
        // The tag says whether the package is installed; it is not a rank.
        let mut hits = vec![
            hit(false, "vimb", "1_1"),
            hit(true, "vim", "1_1"),
            hit(true, "apvlv", "1_1"),
        ];
        rank_search_hits(&mut hits, "vim");
        assert_eq!(names(&hits), vec!["vim", "vimb", "apvlv"]);
    }
}
