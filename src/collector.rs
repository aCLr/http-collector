extern crate select;
use async_trait::async_trait;

use atom_syndication::Feed as AtomFeed;
use futures::future::join_all;
use reqwest::{Client, Response};
use rss::Channel;
use select::document::Document;
use select::predicate::{Attr, Name, Predicate};
use std::str::FromStr;
use url::Url;

use crate::error::{Error, Result};
use crate::models::*;
use tokio::sync::mpsc;

#[async_trait]
pub trait ResultsHandler {
    async fn process(&self, result: Result<(&Feed, FeedKind, String)>);
}

#[async_trait]
pub trait Cache {
    async fn get(&self, link: &str) -> Option<FeedKind>;
    async fn set(&self, link: &str, feed_kind: &FeedKind) -> Result<()>;
}

struct CacheStub {}

#[async_trait]
impl Cache for CacheStub {
    async fn get(&self, link: &str) -> Option<FeedKind> {
        None
    }

    async fn set(&self, link: &str, feed_kind: &FeedKind) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct HttpCollector<C: Cache> {
    client: Client,
    cache: C,
}

impl HttpCollector<CacheStub> {
    pub fn new() -> HttpCollector<CacheStub> {
        HttpCollector {
            client: Client::new(),
            cache: CacheStub {},
        }
    }
}

impl<C> HttpCollector<C>
where
    C: Cache,
{
    pub fn with_cache(&mut self, cache: C) {
        self.cache = cache
    }

    pub async fn run(
        &self,
        mut sources_receiver: mpsc::Receiver<Vec<(Option<FeedKind>, String)>>,
        process_results: &impl ResultsHandler,
    ) {
        while let Some(sources) = sources_receiver.recv().await {
            debug!("retrieve sources: {}", sources.len());
            let mut tasks = vec![];
            for (kind, link) in sources {
                debug!("want to scrape: ({:?}) {}", kind, link);
                tasks.push(self.scrape_and_process_content(kind, link, process_results));
            }
            join_all(tasks).await;
        }
    }

    async fn scrape_and_process_content(
        &self,
        kind: Option<FeedKind>,
        link: String,
        process_results: &impl ResultsHandler,
    ) {
        match self.scrape_feed(kind, link.as_str()).await {
            Ok(content) => {
                process_results
                    .process(Ok((&content, content.kind, link)))
                    .await
            }
            Err(err) => process_results.process(Err(err)).await,
        };
    }

    async fn scrape_unknown_feed_kind(&self, link: &str) -> Result<Feed> {
        let content = self.scrape(link).await?;
        let feeds = self.traverse_parsers(link, content.as_str());
        match feeds.len() {
            0 => Err(Error {
                message: "feeds not found".to_string(),
            }),
            1 => {
                let feed = feeds.first().unwrap();
                Ok(feed.clone())
            }
            _ => {
                let feed = feeds
                    .iter()
                    .find(|f| f.kind == FeedKind::RSS)
                    .or(Some(feeds.first().unwrap()))
                    .unwrap();
                Ok(feed.clone())
            }
        }
    }

    async fn scrape_feed(&self, kind: Option<FeedKind>, link: &str) -> Result<Feed> {
        let kind = match kind {
            None => match self.cache.get(link).await {
                None => {
                    let feed = self.scrape_unknown_feed_kind(link).await?;
                    self.cache.set(link, &feed.kind);
                    return Ok(feed);
                }
                Some(kind) => kind,
            },
            Some(kind) => kind.clone(),
        };
        let result = match kind {
            FeedKind::RSS => self.scrape_rss(link).await?,
            FeedKind::Atom => self.scrape_atom(link).await?,
            FeedKind::WP => Err(Error {
                message: "wp not supported".to_string(),
            })?,
        };
        Ok(result)
    }

    async fn scrape_rss(&self, link: &str) -> Result<Feed> {
        parse_rss_feed(link, self.scrape(link).await?.as_str())
    }

    async fn scrape_atom(&self, link: &str) -> Result<Feed> {
        parse_atom_feed(link, self.scrape(link).await?.as_str())
    }

    async fn scrape(&self, link: &str) -> Result<String> {
        debug!("start scrape {} {:?}", link, std::thread::current().id());
        let response: Response = self.client.get(link).send().await.map_err(|_err| Error {
            message: "can't get text".to_string(),
        })?;
        debug!("scraped {} {:?}", link, std::thread::current().id());
        Ok(response.text().await.map_err(|_err| Error {
            message: "can't get text".to_string(),
        })?)
    }

    async fn detect_possible_feeds(
        &self,
        link: &str,
        scraped_content: &str,
    ) -> Result<Vec<(String, FeedKind)>> {
        let page_scrape_url = Url::parse(link).unwrap();
        let mut for_check: Vec<(String, FeedKind)> = vec![];
        // detect rss content
        let parsed_doc = Document::from_read(scraped_content.as_bytes()).map_err(|_| Error {
            message: "cannot parse html".to_string(),
        })?;

        for (kind, link_type) in vec![
            (FeedKind::RSS, "application/rss+xml"),
            (FeedKind::Atom, "application/atom+xml"),
        ] {
            for element in parsed_doc.find(Name("link").and(Attr("type", link_type))) {
                element.attr("href").map(|href| {
                    let mut link = String::new();
                    if href.starts_with("/") {
                        link.push_str(page_scrape_url.join(href).unwrap().as_str());
                    } else {
                        link.push_str(href);
                    }
                    for_check.push((link.to_string(), kind));
                });
            }
        }

        if let Some(found_wp) = parsed_doc
            .find(Name("link").and(Attr("rel", "https://api.w.org/")))
            .next()
        {
            if let Some(href) = found_wp.attr("href") {
                let parsed_href = Url::parse(href).unwrap();

                for_check.push((
                    parsed_href.join("wp/v2/posts").unwrap().to_string(),
                    FeedKind::WP,
                ));
                for_check.push((
                    page_scrape_url.join("feed/").unwrap().to_string(),
                    FeedKind::RSS,
                ));
            };
        };
        if for_check.is_empty() {
            for element in parsed_doc.find(Attr("href", regex::Regex::new("/rss").unwrap())) {
                element.attr("href").map(|href| {
                    let mut link = String::new();
                    if href.starts_with("/") {
                        link.push_str(page_scrape_url.join(href).unwrap().as_str());
                    } else {
                        link.push_str(href);
                    }
                    for_check.push((link.to_string(), FeedKind::RSS));
                });
            }
        };
        Ok(for_check)
    }

    pub fn traverse_parsers(&self, link: &str, content: &str) -> Vec<Feed> {
        let mut result = vec![];
        let parsers: Vec<&dyn Fn(&str, &str) -> Result<Feed>> =
            vec![&parse_rss_feed, &parse_atom_feed];
        for parser in parsers {
            match parser(link, content) {
                Ok(feed) => {
                    result.push(feed);
                }
                Err(err) => {
                    trace!("not parsed: {}", err);
                }
            }
        }
        result
    }

    pub async fn detect_feeds(&self, link: &str) -> Result<Vec<Feed>> {
        let content = self.scrape(link).await?;

        let mut result = self.traverse_parsers(link, content.as_str());
        let for_check = self.detect_possible_feeds(link, content.as_str()).await?;

        let mut checks = vec![];
        for_check
            .iter()
            .map(|feed| {
                debug!("going to check {:?}", feed);
                checks.push(self.scrape_feed(Some(feed.1), feed.0.as_str()));
            })
            .for_each(drop);
        join_all(checks)
            .await
            .into_iter()
            .map(|check_result| match check_result {
                Ok(feed) => {
                    result.push(feed);
                }
                Err(err) => {
                    error!("{}", err.message.as_str());
                }
            })
            .for_each(drop);
        Ok(result)
    }
}

fn get_image(content: &str) -> Option<String> {
    let parsed_doc = Document::from_read(content.as_bytes());
    match parsed_doc {
        Ok(doc) => {
            let image = match doc.find(Name("img")).next() {
                None => None,
                Some(img) => img.attr("src").map(|x| x.to_string()),
            };
            image
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_get_image() {
        let content = "\
        <p><img src=\"https://habrastorage.org/webt/4n/c1/v0/4nc1v0ifaa8rzyrzq5q1q7r4t8q.png\"></p>
        <br>
        <p>В этой статье я хочу разобрать один из самых популярных опенсорс-инструментов,
        <a href=\"https://nodered.org\" rel=\"nofollow\">Node-RED</a>,\
        с точки зрения создания простых прототипов приложений с минимумом программирования</p>";
        let image = get_image(content);
        assert_eq!(
            image,
            Some(
                "https://habrastorage.org/webt/4n/c1/v0/4nc1v0ifaa8rzyrzq5q1q7r4t8q.png"
                    .to_string()
            )
        )
    }
}

fn current_time() -> chrono::DateTime<chrono::FixedOffset> {
    let local_time = chrono::Local::now();
    let utc_time = chrono::DateTime::<chrono::Utc>::from_utc(local_time.naive_utc(), chrono::Utc);
    utc_time.with_timezone(&chrono::FixedOffset::east(0))
}

fn get_feed_pub_date(pub_date: Option<&str>) -> chrono::NaiveDateTime {
    let pub_date = chrono::DateTime::parse_from_rfc2822(pub_date.unwrap_or_default())
        .unwrap_or(current_time());
    pub_date.naive_utc()
}

fn parse_atom_feed(link: &str, content: &str) -> Result<Feed> {
    let channel = AtomFeed::from_str(content)?;
    let image = match channel.icon() {
        None => None,
        Some(img) => Some(img.to_string()),
    };
    let mut feed_items = vec![];
    for item in channel.entries() {
        let description = item.summary().unwrap_or_default();
        let image = get_image(description);
        feed_items.push(FeedItem {
            title: item.title().to_string(),
            image_link: image,
            pub_date: item.published().unwrap().naive_utc(),
            content: item
                .content()
                .unwrap()
                .value
                .to_owned()
                .unwrap_or_default()
                .to_string(),
            guid: item.id.to_string(),
        })
    }
    Ok(Feed {
        image,
        link: link.to_string(),
        kind: FeedKind::RSS,
        name: channel.title,
        content: feed_items,
    })
}

fn parse_rss_feed(link: &str, content: &str) -> Result<Feed> {
    let channel = Channel::from_str(content)?;
    let mut feed_items: Vec<FeedItem> = vec![];
    for item in channel.items() {
        let description = item.description().unwrap_or_default();
        let mut guid = String::new();
        if item.guid().is_some() {
            guid.push_str(item.guid().unwrap().value())
        } else if item.link().is_some() {
            guid.push_str(item.link().unwrap())
        } else {
            warn!("can't get unique id for record {:?}", item);
            continue;
        }
        feed_items.push(FeedItem {
            title: item.title().unwrap_or_default().to_string(),
            pub_date: get_feed_pub_date(item.pub_date()),
            content: item.content().unwrap_or_default().to_string(),
            guid,
            image_link: get_image(description),
        })
    }
    Ok(Feed {
        image: channel.image().map_or(None, |i| Some(i.url().to_string())),
        link: link.to_string(),
        kind: FeedKind::RSS,
        name: channel.title().to_string(),
        content: feed_items,
    })
}
