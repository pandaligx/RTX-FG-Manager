//! A reviewed game can acquire a new store ID after players have shared setups.
//! Read its existing groups without changing the game ID on a player's card.
use super::*;
use std::collections::HashSet;

impl GameIdentity {
    fn reviewed_entry<'a>(&self, names: &'a game_names::Catalog) -> Option<&'a game_names::Entry> {
        if let Some(id) = self.id.strip_prefix("catalog:") {
            names.entry(&id.to_ascii_uppercase())
        } else {
            names.matched(std::slice::from_ref(&self.id))
        }
    }

    pub fn accepts_group(&self, id: &str, names: &game_names::Catalog) -> bool {
        self.id == id
            || self
                .reviewed_entry(names)
                .is_some_and(|entry| member(entry, id))
    }
}

fn member(entry: &game_names::Entry, id: &str) -> bool {
    id.strip_prefix("catalog:")
        .is_some_and(|q| entry.id().eq_ignore_ascii_case(q))
        || entry.platform_keys().iter().any(|key| key == id)
}

/// Present one choice per reviewed title. Unreviewed identities remain separate
/// even when their display names are identical.
pub fn game_choices(
    names: &game_names::Catalog,
    online: &[GameCandidate],
    local: &[game_names::Entry],
) -> Vec<GameCandidate> {
    let mut result: Vec<GameCandidate> = Vec::new();
    let mut seen = HashSet::new();
    for hit in online
        .iter()
        .cloned()
        .chain(local.iter().filter_map(GameCandidate::from_entry))
    {
        if !seen.insert(hit.identity.id.clone()) {
            continue;
        }
        let reviewed = hit
            .identity
            .reviewed_entry(names)
            .and_then(GameCandidate::from_entry);
        let Some(mut choice) = reviewed else {
            result.push(hit);
            continue;
        };
        if let Some(previous) = result
            .iter_mut()
            .find(|c| c.identity.id == choice.identity.id)
        {
            previous.shared_count = previous.shared_count.saturating_add(hit.shared_count);
        } else {
            choice.shared_count = hit.shared_count;
            result.push(choice);
        }
    }
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeedFilter {
    Public,
    Mine,
    Reported,
}

#[derive(Clone)]
pub struct FeedCursor(Vec<(GameIdentity, u32)>);

pub struct FeedPage {
    pub items: Vec<Card>,
    pub total: u64,
    pub next: Option<FeedCursor>,
    pub notice: Option<&'static str>,
}

fn first_routes(
    game: &GameIdentity,
    entry: &game_names::Entry,
    hits: &[GameCandidate],
) -> FeedCursor {
    let stable = format!("catalog:{}", entry.id().to_ascii_lowercase());
    let mut keys = vec![game.id.as_str(), stable.as_str()];
    if let Some(steam) = entry
        .platform_keys()
        .iter()
        .find(|id| id.starts_with("steam:"))
    {
        keys.push(steam);
    }
    // The service may know more groups, or already serve several keys together.
    // Only locally reviewed IDs may become routes, regardless of title or votes.
    for hit in hits.iter().filter(|hit| hit.shared_count > 0) {
        if let Some(id) = std::iter::once(&hit.identity.id)
            .chain(&hit.keys)
            .find(|id| member(entry, id))
        {
            keys.push(id);
        }
    }
    let mut seen = HashSet::new();
    let routes = keys
        .into_iter()
        .filter_map(|id| {
            let group = hits
                .iter()
                .find(|hit| hit.identity.id == id || hit.keys.iter().any(|k| k == id))
                .map_or(id, |hit| hit.identity.id.as_str());
            seen.insert(group).then(|| {
                (
                    GameIdentity {
                        id: id.to_owned(),
                        name: entry.chinese().to_owned(),
                    },
                    0,
                )
            })
        })
        .collect();
    FeedCursor(routes)
}

impl Client {
    pub fn browse_feeds(
        &self,
        game: &GameIdentity,
        names: &game_names::Catalog,
        catalog: &cloud::Catalog,
        filter: FeedFilter,
        more: Option<FeedCursor>,
    ) -> Result<FeedPage> {
        let first = more.is_none();
        let routes = more.unwrap_or_else(|| {
            if let Some(entry) = game.reviewed_entry(names) {
                // Only a public catalog title leaves the machine on automatic discovery.
                let matches = self.search(entry.chinese()).unwrap_or_default();
                first_routes(game, entry, &matches)
            } else {
                FeedCursor(vec![(game.clone(), 0)])
            }
        });
        let mut pages = Vec::new();
        let mut notice = None;
        let mut failed = None;
        for (group, offset) in routes.0 {
            let page = match self.browse(
                &group,
                catalog,
                offset,
                filter == FeedFilter::Mine,
                filter == FeedFilter::Reported,
            ) {
                Ok(page) => page,
                Err(e) if allows_offline_cache(&e) && first && filter == FeedFilter::Public => {
                    if let Some(page) = self.cached(&group, catalog) {
                        notice = Some(offline_cache_notice(&e));
                        page
                    } else {
                        failed = Some(e);
                        continue;
                    }
                }
                Err(e) => return Err(e),
            };
            pages.push((group, offset, page));
        }
        if let Some(e) = failed {
            if pages.is_empty() {
                return Err(e);
            }
            notice = Some("部分推荐未能加载，请刷新重试");
        }
        let mut result = merge_pages(pages);
        result.notice = notice;
        Ok(result)
    }
}

// Each source is already ordered by votes, update time and ID. Advance only
// past the prefix actually displayed so the next global page cannot skip cards.
fn merge_pages(pages: Vec<(GameIdentity, u32, Page)>) -> FeedPage {
    const PAGE_SIZE: usize = 24;
    let mut streams = Vec::new();
    let mut rows = Vec::new();
    let mut first_ids = HashSet::new();
    let mut total = 0;
    for (game, offset, page) in pages {
        if page
            .items
            .first()
            .is_some_and(|c| !first_ids.insert(c.id.clone()))
        {
            continue; // The service has already combined these routes.
        }
        total += page.total;
        let stream = streams.len();
        streams.push((game, offset, page.items.len(), page.next_offset));
        rows.extend(page.items.into_iter().map(|card| (stream, card)));
    }
    rows.sort_by(|(_, a), (_, b)| card_order(a, b));
    let mut consumed = vec![0; streams.len()];
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for (stream, card) in rows {
        if !seen.contains(&card.id) && items.len() == PAGE_SIZE {
            break;
        }
        consumed[stream] += 1;
        if seen.insert(card.id.clone()) {
            items.push(card);
        }
    }
    let next: Vec<_> = streams
        .into_iter()
        .zip(consumed)
        .filter_map(|((game, offset, len, next), used)| {
            let cursor = if used < len {
                Some(offset + used as u32)
            } else {
                next
            };
            cursor.map(|n| (game, n))
        })
        .collect();
    FeedPage {
        items,
        total,
        next: (!next.is_empty()).then_some(FeedCursor(next)),
        notice: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendations::tests::{card, mock_service, online_test_client};

    const STEAM: &str = "steam:4732690";
    const LEGACY: &str = "catalog:q112049342";
    const EPIC: &str = "epic_slug:arknightsendfield-f0f167";
    const TITLE: &str = "明日方舟：终末地";

    fn identity(id: &str) -> GameIdentity {
        GameIdentity::new(id.into(), TITLE.into()).unwrap()
    }

    fn page(id: &str, votes: impl IntoIterator<Item = u64>) -> Page {
        let items: Vec<_> = votes
            .into_iter()
            .map(|v| {
                let mut c = card("RTX 3060", "", v);
                c.content.game = identity(id);
                c
            })
            .collect();
        Page {
            total: items.len() as u64,
            items,
            next_offset: None,
        }
    }

    #[test]
    fn first_open_collects_both_verified_groups_and_keeps_each_card_route() -> Result<()> {
        let names = game_names::bundled();
        let game: Game =
            serde_json::from_value(json!({"exe":"D:/Games/Endfield.exe","title":TITLE,
            "name_identity":{"keys":[STEAM]}}))?;
        let id = identify(&game, &names).unwrap();
        assert_eq!(id.id, STEAM);
        let directory = json!({"items":[
            {"id":STEAM,"name":TITLE,"keys":[STEAM],"recipes":1},
            {"id":LEGACY,"name":TITLE,"original":"Arknights: Endfield","keys":[LEGACY,EPIC],"recipes":8},
            {"id":"steam:292030","name":TITLE,"keys":["steam:292030"],"recipes":99}]});
        let (origin, server) = mock_service(vec![
            (200, directory),
            (200, json!(page(STEAM, [2]))),
            (200, json!(page(LEGACY, (3..11).rev()))),
        ]);
        let data = tempfile::tempdir()?;
        let client = online_test_client(data.path(), origin);
        let result =
            client.browse_feeds(&id, &names, &cloud::bundled(), FeedFilter::Public, None)?;
        assert_eq!((result.total, result.items.len()), (9, 9));
        assert!(result.next.is_none() && result.notice.is_none());
        assert!(result.items.windows(2).all(|p| p[0].likes >= p[1].likes));
        assert_eq!(
            result
                .items
                .iter()
                .filter(|c| c.content.game.id == LEGACY)
                .count(),
            8
        );
        assert!(
            result
                .items
                .iter()
                .all(|c| GameLink::accepts(&game, &names, &c.content.game.id))
        );
        assert!(!GameLink::accepts(&game, &names, "steam:292030"));
        assert!(recipe_route(&result.items[0], "vote")?.contains("catalog%3Aq112049342"));
        let calls = server.join().unwrap();
        assert!(!calls.iter().any(
            |r| r.contains("Endfield.exe") || r.to_ascii_lowercase().contains("authorization:")
        ));
        Ok(())
    }

    #[test]
    fn joined_server_is_read_once_and_unverified_names_do_not_link() {
        let names = game_names::bundled();
        let id = identity(EPIC);
        let entry = id.reviewed_entry(&names).unwrap();
        let hit = GameCandidate {
            identity: identity(LEGACY),
            keys: vec![STEAM.into(), LEGACY.into(), EPIC.into()],
            shared_count: 9,
            original: String::new(),
            platforms: vec![],
        };
        let cursors = first_routes(&id, entry, &[hit]);
        assert_eq!(cursors.0.len(), 1);
        assert_eq!(cursors.0[0].0.id, EPIC);
        assert_eq!(
            public_search_title(&names, "Arknights: Endfield").as_deref(),
            Some(TITLE)
        );
        assert!(public_search_title(&names, "D:/private/Endfield.exe").is_none());
        assert!(public_search_title(&names, "Arknights: Endfield II Remastered").is_none());
        assert!(
            !GameIdentity::custom(TITLE)
                .unwrap()
                .accepts_group(STEAM, &names)
        );
    }

    #[test]
    fn association_picker_combines_reviewed_ids_but_keeps_unverified_homonyms_separate() {
        let names = game_names::bundled();
        let mut hits: Vec<_> = [(STEAM, 1), (LEGACY, 8)]
            .into_iter()
            .map(|(id, count)| GameCandidate {
                identity: identity(id),
                keys: vec![id.to_owned()],
                original: String::new(),
                platforms: vec![],
                shared_count: count,
            })
            .collect();
        hits.push(GameCandidate {
            identity: GameIdentity::custom(TITLE).unwrap(),
            keys: vec![],
            original: String::new(),
            platforms: vec![],
            shared_count: 2,
        });
        let options = game_choices(&names, &hits, &names.search(TITLE));
        assert_eq!(options.len(), 2);
        assert_eq!(
            (&options[0].identity.id, options[0].shared_count),
            (&STEAM.to_owned(), 9)
        );
        assert_eq!(options[1].shared_count, 2);
    }

    #[test]
    fn interleaved_pages_keep_global_rank_and_advance_only_consumed_items() {
        let a = page(STEAM, (0..31).rev().map(|v| 2 * v));
        let b = page(LEGACY, (0..31).rev().map(|v| 2 * v + 1));
        let mut cursor = FeedCursor(vec![(identity(STEAM), 0), (identity(LEGACY), 0)]);
        let mut votes = vec![];
        loop {
            let pages = cursor
                .0
                .into_iter()
                .map(|(g, offset)| {
                    let p = if g.id == STEAM { &a } else { &b };
                    let end = (offset as usize + 24).min(p.items.len());
                    let part = Page {
                        items: p.items[offset as usize..end].to_vec(),
                        total: p.total,
                        next_offset: (end < p.items.len()).then_some(end as u32),
                    };
                    (g, offset, part)
                })
                .collect();
            let loaded = merge_pages(pages);
            votes.extend(loaded.items.iter().map(|c| c.likes));
            let Some(next) = loaded.next else { break };
            cursor = next;
        }
        assert_eq!(votes, (0..62).rev().collect::<Vec<_>>());
        let merged = merge_pages(vec![
            (identity(STEAM), 0, b.clone()),
            (identity(LEGACY), 0, b),
        ]);
        assert_eq!(merged.total, 31);
        assert_eq!(merged.items.len(), 24);
    }

    #[test]
    #[ignore = "Read-only public recommendation service; run explicitly"]
    fn public_endfield() -> Result<()> {
        let data = tempfile::tempdir()?;
        let client = Client::new(data.path())?;
        let names = game_names::bundled();
        let catalog = cloud::bundled();
        let hits = client.search(TITLE)?;
        let routes = first_routes(
            &identity(STEAM),
            identity(STEAM).reviewed_entry(&names).unwrap(),
            &hits,
        );
        let mut ids = HashSet::new();
        for (group, _) in routes.0 {
            let mut offset = 0;
            loop {
                let page = client.browse(&group, &catalog, offset, false, false)?;
                ids.extend(page.items.into_iter().map(|c| c.id));
                let Some(next) = page.next_offset else { break };
                offset = next;
            }
        }
        let mut loaded = HashSet::new();
        let mut more = None;
        loop {
            let page = client.browse_feeds(
                &identity(STEAM),
                &names,
                &catalog,
                FeedFilter::Public,
                more,
            )?;
            assert!(page.notice.is_none());
            loaded.extend(page.items.into_iter().map(|c| c.id));
            more = page.next;
            if more.is_none() {
                break;
            }
        }
        assert!(!ids.is_empty());
        assert_eq!(loaded, ids);
        println!("Verified {} distinct public setups.", loaded.len());
        Ok(())
    }
}
