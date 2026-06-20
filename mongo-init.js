db = db.getSiblingDB("crasp");

db.pages.createIndex({ crawl_id: 1, depth: 1 });
db.pages.createIndex({ crawl_id: 1, url_normalized: 1 }, { unique: true });
db.pages.createIndex({ crawl_id: 1, status: 1 });
db.pages.createIndex({ duplicate_group_id: 1 });
db.pages.createIndex({ timestamp: -1 });

db.crawls.createIndex({ crawl_id: 1 }, { unique: true });
db.crawls.createIndex({ started_at: -1 });

db.content_hashes.createIndex({ hash: 1, hash_algorithm: 1 }, { unique: true });
db.content_hashes.createIndex({ first_seen_crawl_id: 1 });
