//! Only same-origin hints with independently accepted, duplicate content become
//! aliases. All changes occur inside the existing publication transaction.
use rusqlite::{params, Transaction};
use std::collections::HashMap;

pub(crate) fn consolidate(tx: &Transaction<'_>) -> anyhow::Result<()> {
    let edges = tx.prepare("SELECT c.source,c.target FROM crawl_canonicals c
        JOIN staging_pages s ON s.url=c.source JOIN staging_pages t ON t.url=c.target
        WHERE c.source<>c.target AND COALESCE(s.text,'')=COALESCE(t.text,'')
        AND (length(trim(COALESCE(s.text,'')))>0 OR
            (COALESCE(s.title,'')=COALESCE(t.title,'') AND COALESCE(s.description,'')=COALESCE(t.description,'') AND length(trim(COALESCE(s.title,'')))>0))")?
        .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
        .collect::<rusqlite::Result<HashMap<_,_>>>()?;
    let edges: HashMap<_, _> = edges
        .into_iter()
        .filter(
            |(source, target)| match (url::Url::parse(source), url::Url::parse(target)) {
                (Ok(s), Ok(t)) => s.scheme() == "https" && s.origin() == t.origin(),
                _ => false,
            },
        )
        .collect();
    let aliases = resolve(&edges);
    tx.execute("DELETE FROM crawl_aliases", [])?;
    for (alias, target) in aliases {
        if alias != target {
            tx.execute(
                "INSERT INTO crawl_aliases(alias,target) VALUES(?1,?2)",
                params![alias, target],
            )?;
        }
    }
    tx.execute_batch("DELETE FROM pages WHERE url IN (SELECT alias FROM crawl_aliases);
        UPDATE images SET referring_url=COALESCE((SELECT target FROM crawl_aliases WHERE alias=images.referring_url),referring_url);
        INSERT OR IGNORE INTO news(url,title,domain,published_at,description)
            SELECT a.target,n.title,n.domain,n.published_at,n.description FROM news n JOIN crawl_aliases a ON a.alias=n.url ORDER BY n.url;
        DELETE FROM news WHERE url IN (SELECT alias FROM crawl_aliases);
        INSERT OR IGNORE INTO videos(url,title,domain,thumbnail_url,published_at,description)
            SELECT a.target,v.title,v.domain,v.thumbnail_url,v.published_at,v.description FROM videos v JOIN crawl_aliases a ON a.alias=v.url ORDER BY v.url;
        DELETE FROM videos WHERE url IN (SELECT alias FROM crawl_aliases);")?;
    Ok(())
}

fn resolve(edges: &HashMap<String, String>) -> HashMap<String, String> {
    let mut resolved = HashMap::<String, String>::new();
    for source in edges.keys() {
        if resolved.contains_key(source) {
            continue;
        }
        let mut path = Vec::<String>::new();
        let mut visited = HashMap::new();
        let mut node = source.clone();
        let representative = loop {
            if let Some(target) = resolved.get(&node) {
                break target.clone();
            }
            if let Some(&start) = visited.get(&node) {
                break path[start..].iter().min().unwrap().clone();
            }
            visited.insert(node.clone(), path.len());
            path.push(node.clone());
            match edges.get(&node) {
                Some(next) => node = next.clone(),
                None => break node,
            }
        };
        for alias in path {
            resolved.insert(alias, representative.clone());
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chains_and_cycles_have_stable_representatives() {
        let edges = HashMap::from([
            ("a".into(), "b".into()),
            ("b".into(), "c".into()),
            ("x".into(), "z".into()),
            ("z".into(), "y".into()),
            ("y".into(), "z".into()),
        ]);
        let aliases = resolve(&edges);
        assert_eq!(aliases["a"], "c");
        assert_eq!(aliases["b"], "c");
        assert_eq!(aliases["x"], "y");
        assert_eq!(aliases["z"], "y");
        assert_eq!(aliases["y"], "y");
    }
}
