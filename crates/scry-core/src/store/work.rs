use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::Store;
use crate::{Error, Result};

pub const WORK_STATUSES: &[&str] = &["todo", "doing", "done", "dropped"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: i64,
    pub task: String,
    pub status: String,
    pub note: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkList {
    pub name: String,
    pub closed: bool,
    pub updated_at: i64,
    pub state: Vec<(String, String)>,
    pub items: Vec<WorkItem>,
}

fn validate_status(status: &str) -> Result<()> {
    if WORK_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(Error::Config(format!(
        "unknown status '{status}'; expected one of {}",
        WORK_STATUSES.join(", ")
    )))
}

/// The id of the named list, created if missing. Any write to a list
/// reopens it.
fn touch_list(conn: &Connection, repo_id: i64, name: &str) -> Result<i64> {
    Ok(conn.query_row(
        "INSERT INTO work_lists (repo_id, name) VALUES (?1, ?2)
         ON CONFLICT (repo_id, name) DO UPDATE SET closed_at = NULL, updated_at = unixepoch()
         RETURNING id",
        params![repo_id, name],
        |row| row.get(0),
    )?)
}

/// Moves a repo's work lists to `target`, folding a list into the target's
/// same-named one where both exist; without a target they are deleted.
pub(super) fn move_work(conn: &Connection, repo_id: i64, target: Option<i64>) -> Result<()> {
    if let Some(target) = target {
        let twin = "SELECT t.id FROM work_lists t JOIN work_lists f ON f.name = t.name
                    WHERE t.repo_id = ?2 AND f.id = list_id";
        let colliding = "SELECT f.id FROM work_lists f JOIN work_lists t ON t.name = f.name
                         WHERE f.repo_id = ?1 AND t.repo_id = ?2";
        conn.execute(
            &format!("UPDATE work_items SET list_id = ({twin}) WHERE list_id IN ({colliding})"),
            params![repo_id, target],
        )?;
        conn.execute(
            &format!(
                "UPDATE OR IGNORE work_state SET list_id = ({twin}) WHERE list_id IN ({colliding})"
            ),
            params![repo_id, target],
        )?;
        conn.execute(
            &format!("DELETE FROM work_state WHERE list_id IN ({colliding})"),
            params![repo_id, target],
        )?;
        conn.execute(
            &format!("DELETE FROM work_lists WHERE id IN ({colliding})"),
            params![repo_id, target],
        )?;
        conn.execute(
            "UPDATE work_lists SET repo_id = ?2 WHERE repo_id = ?1",
            params![repo_id, target],
        )?;
        return Ok(());
    }
    let owned = "SELECT id FROM work_lists WHERE repo_id = ?1";
    conn.execute(
        &format!("DELETE FROM work_state WHERE list_id IN ({owned})"),
        [repo_id],
    )?;
    conn.execute(
        &format!("DELETE FROM work_items WHERE list_id IN ({owned})"),
        [repo_id],
    )?;
    conn.execute("DELETE FROM work_lists WHERE repo_id = ?1", [repo_id])?;
    Ok(())
}

impl Store {
    pub fn add_work_item(
        &mut self,
        repo_id: i64,
        list: &str,
        task: &str,
        status: &str,
        note: Option<&str>,
    ) -> Result<i64> {
        validate_status(status)?;
        let tx = self.conn.transaction()?;
        let list_id = touch_list(&tx, repo_id, list)?;
        tx.execute(
            "INSERT INTO work_items (list_id, task, status, note) VALUES (?1, ?2, ?3, ?4)",
            params![list_id, task, status, note],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Sets an item's status and/or note; `None` leaves a field as is.
    /// Ids from another repo's lists are refused.
    pub fn update_work_item(
        &mut self,
        repo_id: i64,
        id: i64,
        status: Option<&str>,
        note: Option<&str>,
    ) -> Result<()> {
        status.map(validate_status).transpose()?;
        let tx = self.conn.transaction()?;
        let list_id: i64 = tx
            .query_row(
                "UPDATE work_items SET status = coalesce(?2, status), note = coalesce(?3, note),
                     updated_at = unixepoch()
                 WHERE id = ?1 AND list_id IN (SELECT id FROM work_lists WHERE repo_id = ?4)
                 RETURNING list_id",
                params![id, status, note, repo_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Config(format!("no work item #{id}")))?;
        tx.execute(
            "UPDATE work_lists SET closed_at = NULL, updated_at = unixepoch() WHERE id = ?1",
            [list_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Sets state keys on a list; a `None` value clears the key.
    pub fn set_work_state(
        &mut self,
        repo_id: i64,
        list: &str,
        entries: &[(String, Option<String>)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let list_id = touch_list(&tx, repo_id, list)?;
        for (key, value) in entries {
            match value {
                Some(value) => tx.execute(
                    "INSERT INTO work_state (list_id, key, value) VALUES (?1, ?2, ?3)
                     ON CONFLICT (list_id, key) DO UPDATE SET value = excluded.value",
                    params![list_id, key, value],
                )?,
                None => tx.execute(
                    "DELETE FROM work_state WHERE list_id = ?1 AND key = ?2",
                    params![list_id, key],
                )?,
            };
        }
        tx.commit()?;
        Ok(())
    }

    pub fn close_work_list(&mut self, repo_id: i64, list: &str, closed: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE work_lists SET closed_at = CASE WHEN ?3 THEN unixepoch() END,
                 updated_at = unixepoch()
             WHERE repo_id = ?1 AND name = ?2",
            params![repo_id, list, closed],
        )?;
        if changed == 0 {
            return Err(Error::Config(format!("no work list '{list}'")));
        }
        Ok(())
    }

    /// Lists in scope, most recently updated first, each with its state and
    /// open items. `all` adds closed lists and finished items; naming a list
    /// returns it even when closed.
    pub fn work_lists(&self, repo_id: i64, name: Option<&str>, all: bool) -> Result<Vec<WorkList>> {
        let mut lists = self.conn.prepare(
            "SELECT id, name, closed_at IS NOT NULL, updated_at FROM work_lists
             WHERE repo_id = ?1 AND (?2 IS NULL OR name = ?2)
               AND (?3 OR ?2 IS NOT NULL OR closed_at IS NULL)
             ORDER BY updated_at DESC, id DESC",
        )?;
        let rows: Vec<(i64, String, bool, i64)> = lists
            .query_map(params![repo_id, name, all], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(id, name, closed, updated_at)| {
                Ok(WorkList {
                    name,
                    closed,
                    updated_at,
                    state: self.work_state(id)?,
                    items: self.work_items(id, all)?,
                })
            })
            .collect()
    }

    fn work_state(&self, list_id: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM work_state WHERE list_id = ?1 ORDER BY key")?;
        let state = stmt
            .query_map([list_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(state)
    }

    fn work_items(&self, list_id: i64, all: bool) -> Result<Vec<WorkItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task, status, note, updated_at FROM work_items
             WHERE list_id = ?1 AND (?2 OR status IN ('todo', 'doing'))
             ORDER BY CASE status WHEN 'doing' THEN 0 WHEN 'todo' THEN 1
                                  WHEN 'done' THEN 2 ELSE 3 END, id",
        )?;
        let items = stmt
            .query_map(params![list_id, all], |row| {
                Ok(WorkItem {
                    id: row.get(0)?,
                    task: row.get(1)?,
                    status: row.get(2)?,
                    note: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_repo() -> (Store, i64) {
        let store = Store::open_in_memory("m", 4).unwrap();
        let repo = store.upsert_repo("github.com/x/y").unwrap();
        (store, repo)
    }

    #[test]
    fn open_lists_show_open_items_and_state() {
        let (mut store, repo) = store_with_repo();
        let first = store
            .add_work_item(repo, "ship", "write tests", "todo", None)
            .unwrap();
        let second = store
            .add_work_item(repo, "ship", "implement", "doing", None)
            .unwrap();
        store
            .update_work_item(repo, first, Some("done"), Some("all green"))
            .unwrap();
        store
            .set_work_state(repo, "ship", &[("pr".into(), Some("#12".into()))])
            .unwrap();

        let lists = store.work_lists(repo, None, false).unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].state, vec![("pr".to_string(), "#12".to_string())]);
        let ids: Vec<i64> = lists[0].items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![second]);

        let all = store.work_lists(repo, Some("ship"), true).unwrap();
        assert_eq!(all[0].items.len(), 2);
        assert_eq!(all[0].items[1].note.as_deref(), Some("all green"));
    }

    #[test]
    fn state_none_clears_a_key_and_unknown_status_is_refused() {
        let (mut store, repo) = store_with_repo();
        store
            .set_work_state(repo, "l", &[("next".into(), Some("x".into()))])
            .unwrap();
        store
            .set_work_state(repo, "l", &[("next".into(), None)])
            .unwrap();
        assert!(
            store.work_lists(repo, None, false).unwrap()[0]
                .state
                .is_empty()
        );
        assert!(
            store
                .add_work_item(repo, "l", "t", "blocked", None)
                .is_err()
        );
        assert!(
            store
                .update_work_item(repo, 999, Some("done"), None)
                .is_err()
        );
        let other = store.upsert_repo("github.com/x/other").unwrap();
        let foreign = store.add_work_item(other, "l", "t", "todo", None).unwrap();
        assert!(
            store
                .update_work_item(repo, foreign, Some("done"), None)
                .is_err()
        );
    }

    #[test]
    fn closed_lists_hide_until_named_or_written() {
        let (mut store, repo) = store_with_repo();
        let id = store.add_work_item(repo, "l", "t", "todo", None).unwrap();
        store.close_work_list(repo, "l", true).unwrap();
        assert!(store.work_lists(repo, None, false).unwrap().is_empty());
        assert!(store.work_lists(repo, Some("l"), false).unwrap()[0].closed);
        assert_eq!(store.work_lists(repo, None, true).unwrap().len(), 1);

        store
            .update_work_item(repo, id, None, Some("back on it"))
            .unwrap();
        assert_eq!(store.work_lists(repo, None, false).unwrap().len(), 1);
        assert!(store.close_work_list(repo, "missing", true).is_err());
    }

    #[test]
    fn prune_into_folds_same_named_lists() {
        let (mut store, old) = store_with_repo();
        let new = store.upsert_repo("github.com/x/z").unwrap();
        store
            .add_work_item(old, "shared", "from old", "todo", None)
            .unwrap();
        store
            .add_work_item(old, "solo", "only old", "todo", None)
            .unwrap();
        store
            .add_work_item(new, "shared", "from new", "todo", None)
            .unwrap();
        store
            .set_work_state(old, "shared", &[("pr".into(), Some("old".into()))])
            .unwrap();
        store
            .set_work_state(new, "shared", &[("pr".into(), Some("new".into()))])
            .unwrap();

        store.prune_repo("github.com/x/y", Some(new)).unwrap();
        let lists = store.work_lists(new, None, false).unwrap();
        let shared = lists.iter().find(|l| l.name == "shared").unwrap();
        assert_eq!(shared.items.len(), 2);
        assert_eq!(shared.state, vec![("pr".to_string(), "new".to_string())]);
        assert!(lists.iter().any(|l| l.name == "solo"));
    }

    #[test]
    fn prune_without_target_deletes_work() {
        let (mut store, repo) = store_with_repo();
        store.add_work_item(repo, "l", "t", "todo", None).unwrap();
        store.prune_repo("github.com/x/y", None).unwrap();
        let left: i64 = store
            .conn
            .query_row("SELECT count(*) FROM work_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }
}
