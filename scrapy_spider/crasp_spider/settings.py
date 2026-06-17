import os

BOT_NAME = "crasp_spider"

SPIDER_MODULES = ["crasp_spider.spiders"]
NEWSPIDER_MODULE = "crasp_spider.spiders"

ROBOTSTXT_OBEY = False

FEED_URI = os.environ.get("CRASP_FEED_URI", "")
if FEED_URI:
    FEEDS = {FEED_URI: {"format": "jsonlines"}}
else:
    FEEDS = {}

REQUEST_FINGERPRINTER_IMPLEMENTATION = "2.7"
TWISTED_REACTOR = "twisted.internet.asyncioreactor.AsyncioSelectorReactor"

CONCURRENT_REQUESTS = 8
DOWNLOAD_TIMEOUT = 30

LOG_LEVEL = "INFO"
