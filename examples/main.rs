use async_trait::async_trait;
use chrono::NaiveDateTime;
use http_collector::collector::{HttpCollector, ResultsHandler};
use http_collector::models::{Feed, FeedItem, FeedKind};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug)]
pub struct ForFeedItem {
    pub title: String,
    pub content: String,
    pub pub_date: NaiveDateTime,
    pub guid: String,
    pub image_link: Option<String>,
}

impl From<&FeedItem> for ForFeedItem {
    fn from(fi: &FeedItem) -> Self {
        Self {
            title: fi.title.to_string(),
            content: fi.content.to_string(),
            pub_date: fi.pub_date,
            guid: fi.guid.to_string(),
            image_link: match &fi.image_link {
                None => None,
                Some(l) => Some(l.to_string()),
            },
        }
    }
}

#[derive(Debug)]
pub struct ForFeed {
    pub image: Option<String>,
    pub link: String,
    pub kind: FeedKind,
    pub name: String,
    pub content: Vec<ForFeedItem>,
}

impl From<Feed> for ForFeed {
    fn from(feed: Feed) -> Self {
        Self {
            image: feed.image,
            link: feed.link,
            kind: feed.kind,
            name: feed.name,
            content: feed.content.iter().map(ForFeedItem::from).collect(),
        }
    }
}

#[tokio::main]
async fn main() {
    let (sender, mut receiver) = mpsc::channel::<(Feed, FeedKind, String)>(2000);
    let sender = Arc::new(Mutex::new(sender));

    let proc = Handler::new(sender);
    let collector = Arc::new(HttpCollector::new());
    let crun = collector.clone();
    tokio::spawn(async move { crun.run(&get_sources, &proc, &3).await });
    let mut x: i32 = 1;
    while x >= 0 {
        println!("{:?}", receiver.recv().await);
        x -= 1
    }
    let detected = collector.detect_feeds("https://google.com").await;
    println!("{:?}", detected);
}

pub struct Handler {
    channel: Arc<Mutex<mpsc::Sender<(Feed, FeedKind, String)>>>,
}

impl Handler {
    pub fn new(channel: Arc<Mutex<mpsc::Sender<(Feed, FeedKind, String)>>>) -> Self {
        Self { channel }
    }
}

#[async_trait]
impl ResultsHandler for Handler {
    async fn process(&self, update: &Feed, feed_kind: FeedKind, link: String) {
        let mut local = self.channel.lock().await;
        local.send((update.clone(), feed_kind, link)).await.unwrap();
    }
}

fn get_sources() -> Vec<(FeedKind, String)> {
    vec![
        (
            FeedKind::RSS,
            "https://habr.com/ru/rss/best/daily/?fl=ru".to_string(),
        ),
        (FeedKind::WP, "https://google.com".to_string()),
        (FeedKind::Atom, "https://google.com".to_string()),
    ]
}
