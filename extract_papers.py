#!/usr/bin/env python3
"""Extract entities and events from Qdrant papers collection"""

import json
import requests
from datetime import datetime
from collections import defaultdict
import re

# Qdrant configuration
QDRANT_URL = "http://localhost:6333"
COLLECTION_NAME = "papers"

def extract_all_papers():
    """Extract all papers from Qdrant collection"""
    papers = []
    offset = None
    
    while True:
        # Scroll through collection
        payload = {
            "limit": 100,
            "with_payload": True,
            "with_vector": False
        }
        if offset:
            payload["offset"] = offset
            
        response = requests.post(
            f"{QDRANT_URL}/collections/{COLLECTION_NAME}/points/scroll",
            json=payload
        )
        
        result = response.json()
        batch = result.get("result", {}).get("points", [])
        
        if not batch:
            break
            
        papers.extend(batch)
        offset = result.get("result", {}).get("next_page_offset")
        
        if not offset:
            break
    
    return papers

def extract_year_from_url(url):
    """Extract year from arXiv URL"""
    if "arxiv.org" in url:
        # Match patterns like /abs/2502.19614 or /abs/2402.12345
        match = re.search(r'/abs/(\d{2})(\d{2})', url)
        if match:
            year_prefix = int(match.group(1))
            # arXiv uses YYMM format, years 00-99 map to 2000-2099
            if year_prefix >= 0:
                return 2000 + year_prefix
    return None

def process_papers_to_entities_and_events(papers):
    """Process papers to extract entities and events"""
    entities = []
    events = []
    
    # Track unique entities
    seen_entities = set()
    author_papers = defaultdict(list)
    
    for paper in papers:
        payload = paper.get("payload", {})
        
        # Extract basic info
        title = payload.get("title", "")
        authors = payload.get("authors", [])
        categories = payload.get("categories", [])
        abstract = payload.get("abstract", "")
        url = payload.get("url", "")
        paper_id = paper.get("id", "")
        
        # Extract year from URL
        year = extract_year_from_url(url)
        
        # Process authors as entities
        for author in authors:
            if author and author not in seen_entities:
                seen_entities.add(author)
                author_papers[author].append(title)
        
        # Create event for paper publication
        if title and year:
            event_id = f"{year}-paper-{paper_id}"
            event = {
                "id": event_id,
                "timestamp": f"{year}-01-01T00:00:00Z",
                "description": f"Paper published: '{title}' by {', '.join(authors[:3])}{'...' if len(authors) > 3 else ''}",
                "category": "research-publication"
            }
            events.append(event)
        
        # Extract model/system names from title and abstract
        text = title + " " + abstract
        
        # Common patterns for models and systems
        model_patterns = [
            r'\b([A-Z][a-zA-Z]*(?:[A-Z][a-zA-Z]*)+)\b',  # CamelCase
            r'\b([A-Z][A-Z0-9\-]+[A-Z0-9])\b',  # UPPERCASE-NAMES
            r'\b([A-Za-z]+(?:Net|Model|System|Framework|Engine))\b',  # *Net, *Model, etc
        ]
        
        potential_models = set()
        for pattern in model_patterns:
            matches = re.findall(pattern, text)
            potential_models.update(matches)
        
        # Filter common words and add as entities
        common_words = {'The', 'This', 'These', 'That', 'Those', 'Our', 'We', 'In', 'On', 'At', 'For', 'With', 'From', 'To', 'Of', 'And', 'Or', 'But', 'Not'}
        for model in potential_models:
            if model not in common_words and len(model) > 2 and model not in seen_entities:
                # Check if it's mentioned multiple times (likely important)
                if text.count(model) >= 2:
                    seen_entities.add(model)
                    entity = {
                        "name": model,
                        "summary": f"Model/System mentioned in: {title[:100]}...",
                        "tags": ["ai-model", "research"] + [cat.replace("cs.", "") for cat in categories[:3]]
                    }
                    entities.append(entity)
    
    # Create author entities with their papers
    for author, author_titles in author_papers.items():
        entity = {
            "name": author,
            "summary": f"Researcher with {len(author_titles)} paper(s) in the collection. Recent work includes: {author_titles[0][:100]}...",
            "tags": ["researcher", "ai-researcher", "author"]
        }
        entities.append(entity)
    
    # Add category-based events
    category_counts = defaultdict(int)
    for paper in papers:
        for cat in paper.get("payload", {}).get("categories", []):
            category_counts[cat] += 1
    
    # Create milestone events for major categories
    for category, count in category_counts.items():
        if count >= 50:  # Significant number of papers
            event = {
                "id": f"2024-category-milestone-{category.replace('.', '-')}",
                "timestamp": "2024-01-01T00:00:00Z",
                "description": f"Research milestone: {count} papers in {category} category demonstrating significant activity in this field",
                "category": "research-milestone"
            }
            events.append(event)
    
    return entities, events

def main():
    print("Extracting papers from Qdrant...")
    papers = extract_all_papers()
    print(f"Found {len(papers)} papers")
    
    print("Processing papers to extract entities and events...")
    entities, events = process_papers_to_entities_and_events(papers)
    
    print(f"Extracted {len(entities)} entities and {len(events)} events")
    
    # Load existing data
    with open("data/entities.json", "r") as f:
        existing_entities = json.load(f)
    
    with open("data/events.json", "r") as f:
        existing_events = json.load(f)
    
    # Merge with existing data (avoiding duplicates)
    existing_entity_names = {e["name"] for e in existing_entities}
    existing_event_ids = {e["id"] for e in existing_events}
    
    new_entities = [e for e in entities if e["name"] not in existing_entity_names]
    new_events = [e for e in events if e["id"] not in existing_event_ids]
    
    # Combine and save
    all_entities = existing_entities + new_entities
    all_events = existing_events + new_events
    
    # Sort for consistency
    all_entities.sort(key=lambda x: x["name"])
    all_events.sort(key=lambda x: x["timestamp"], reverse=True)
    
    # Save updated data
    with open("data/entities.json", "w") as f:
        json.dump(all_entities, f, indent=2)
    
    with open("data/events.json", "w") as f:
        json.dump(all_events, f, indent=2)
    
    print(f"Total entities: {len(all_entities)} ({len(new_entities)} new)")
    print(f"Total events: {len(all_events)} ({len(new_events)} new)")
    print("Data saved to data/entities.json and data/events.json")

if __name__ == "__main__":
    main()