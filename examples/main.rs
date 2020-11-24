use async_trait::async_trait;
use chrono::NaiveDateTime;
use http_collector::collector::{HttpCollector, ResultsHandler};
use http_collector::models::{Feed, FeedItem, FeedKind};
use http_collector::result::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug)]
pub struct ForFeedItem {
    pub title: Option<String>,
    pub content: String,
    pub pub_date: NaiveDateTime,
    pub guid: String,
    pub image_link: Option<String>,
}

impl From<&FeedItem> for ForFeedItem {
    fn from(fi: &FeedItem) -> Self {
        Self {
            title: fi.title.clone(),
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
    let (sender, mut receiver) = mpsc::channel::<Result<(Feed, FeedKind, String)>>(2000);
    let sender = Arc::new(Mutex::new(sender));

    let proc = Handler::new(sender);
    let collector = Arc::new(HttpCollector::new());
    let crun = collector.clone();
    let (mut sources_sender, sources_receiver) =
        mpsc::channel::<Vec<(Option<FeedKind>, String)>>(2000);
    tokio::spawn(async move { crun.run(sources_receiver, &proc).await });
    tokio::spawn(async move {
        loop {
            sources_sender.send(get_sources()).await.unwrap();
            sources_sender.send(get_sources()).await.unwrap();
            sources_sender.send(get_sources()).await.unwrap();
            sources_sender.send(get_sources()).await.unwrap();
            sources_sender.send(get_sources()).await.unwrap();
        }
    });
    let mut x: i32 = 7;
    while x >= 0 {
        println!("{:?}", receiver.recv().await);
        x -= 1
    }
    let detected =
        tokio::spawn(async move { collector.detect_feeds("https://google.com").await }).await;
    println!("{:?}", detected);
}

pub struct Handler {
    channel: Arc<Mutex<mpsc::Sender<Result<(Feed, FeedKind, String)>>>>,
}

impl Handler {
    pub fn new(channel: Arc<Mutex<mpsc::Sender<Result<(Feed, FeedKind, String)>>>>) -> Self {
        Self { channel }
    }
}

#[async_trait]
impl ResultsHandler for Handler {
    async fn process(&self, result: Result<(&Feed, FeedKind, String)>) {
        let update = match result {
            Ok((updates, kind, link)) => Ok((updates.clone(), kind, link)),
            Err(err) => Err(err),
        };
        let mut local = self.channel.lock().await;
        local.send(update).await.unwrap();
    }
}

fn get_sources() -> Vec<(Option<FeedKind>, String)> {
    vec![
        (
            Some(FeedKind::RSS),
            "https://habr.com/ru/rss/best/daily/?fl=ru".to_string(),
        ),
        (Some(FeedKind::WP), "https://google.com".to_string()),
        (Some(FeedKind::Atom), "https://google.com".to_string()),
        (
            None,
            "https://habr.com/ru/rss/best/daily/?fl=ru".to_string(),
        ),
    ]
}
