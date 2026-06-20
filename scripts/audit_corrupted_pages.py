#!/usr/bin/env python3
"""
WI-34: Best-effort, honest audit of already-corrupted Mongo data.

This script finds PageDoc records where `status == "Completed"` but
`status_code` falls outside the 200-299 range, suggesting the status
was likely mislabeled by the old bug that defaulted missing/structured
status fields to "Completed".

Run against a test or production MongoDB. The script is read-only by
default; use --write-flag to set `status_possibly_corrupted: true` on
matching documents so a human can review them later.

Usage:
    python audit_corrupted_pages.py --uri mongodb://localhost:27017 --write-flag
"""

import argparse
import sys


def main():
    parser = argparse.ArgumentParser(description="Audit corrupted page status in MongoDB")
    parser.add_argument("--uri", default="mongodb://localhost:27017", help="MongoDB URI")
    parser.add_argument("--write-flag", action="store_true", help="Set status_possibly_corrupted=true on matches")
    args = parser.parse_args()

    try:
        import pymongo
    except ImportError:
        print("Error: pymongo is required. Install with: pip install pymongo")
        sys.exit(1)

    client = pymongo.MongoClient(args.uri)
    db = client["crasp"]
    pages = db["pages"]

    # Find documents where status is "Completed" but status_code
    # indicates a non-successful HTTP response.
    query = {
        "status": "Completed",
        "status_code": {"$exists": True, "$not": {"$gte": 200, "$lte": 299}},
    }

    cursor = pages.find(query, {"url": 1, "status": 1, "status_code": 1, "title": 1, "timestamp": 1})
    matches = list(cursor)

    print(f"Found {len(matches)} potentially corrupted documents (status='Completed' with non-2xx status_code)")
    print()

    for doc in matches:
        print(f"  - {doc.get('url', 'N/A')}")
        print(f"      status={doc.get('status')}, status_code={doc.get('status_code')}")
        print(f"      title={doc.get('title', 'N/A')[:60]}")
        print(f"      timestamp={doc.get('timestamp', 'N/A')}")
        print()

    if args.write_flag and matches:
        ids = [doc["_id"] for doc in matches if "_id" in doc]
        result = pages.update_many(
            {"_id": {"$in": ids} if len(ids) > 1 else {"$eq": ids[0]}} if ids else {"_id": None},
            {"$set": {"status_possibly_corrupted": True}}
        )
        print(f"Marked {result.modified_count} documents with 'status_possibly_corrupted: true'")
    elif args.write_flag:
        print("No documents to flag.")
    else:
        print("This was a read-only audit. Use --write-flag to persist the flag.")

    client.close()


if __name__ == "__main__":
    main()
