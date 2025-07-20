#!/usr/bin/env python3
"""Enhanced extraction of entities and events from papers"""

import json
import requests
import re
from collections import defaultdict, Counter
from datetime import datetime

QDRANT_URL = "http://localhost:6333"
COLLECTION_NAME = "papers"

# Known important models and systems
KNOWN_MODELS = {
    "GPT", "BERT", "RoBERTa", "DALL-E", "CLIP", "Transformer", "ResNet", "YOLO",
    "ViT", "Diffusion", "GAN", "VAE", "CNN", "RNN", "LSTM", "GRU", "T5", "BART",
    "LLaMA", "Alpaca", "Vicuna", "Claude", "Gemini", "PaLM", "LaMDA", "Mistral",
    "Stable Diffusion", "ControlNet", "LoRA", "QLoRA", "PEFT", "Whisper",
    "SAM", "DINO", "MAE", "SimCLR", "BYOL", "SwAV", "MoCo", "DeiT", "Swin",
    "ConvNeXt", "EfficientNet", "MobileNet", "SqueezeNet", "DenseNet", "Inception",
    "AlexNet", "VGG", "U-Net", "Mask R-CNN", "Fast R-CNN", "Faster R-CNN",
    "RetinaNet", "CenterNet", "DETR", "YOLOv", "SSD", "R-CNN"
}

# Important institutions and labs
KNOWN_INSTITUTIONS = {
    "OpenAI", "Anthropic", "DeepMind", "Google Research", "Google Brain",
    "Meta AI", "Facebook AI", "FAIR", "Microsoft Research", "MSR",
    "MIT", "Stanford", "Berkeley", "CMU", "Cornell", "Princeton",
    "Harvard", "Yale", "Oxford", "Cambridge", "ETH Zurich", "EPFL",
    "Max Planck", "INRIA", "MILA", "Vector Institute", "AI2",
    "Hugging Face", "Stability AI", "Midjourney", "Runway ML",
    "NVIDIA", "AMD", "Intel", "IBM Research", "Amazon Science",
    "Apple ML", "Tesla AI", "Baidu Research", "Alibaba DAMO",
    "Tencent AI Lab", "ByteDance", "JD AI", "Samsung Research"
}

def extract_all_papers():
    """Extract all papers from Qdrant"""
    papers = []
    offset = None
    
    while True:
        payload = {"limit": 100, "with_payload": True, "with_vector": False}
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
        match = re.search(r'/abs/(\d{2})(\d{2})', url)
        if match:
            year_prefix = int(match.group(1))
            return 2000 + year_prefix
    return None

def extract_models_from_text(text):
    """Extract model names from text"""
    models = set()
    
    # Check for known models
    for model in KNOWN_MODELS:
        if re.search(rf'\b{model}\b', text, re.IGNORECASE):
            models.add(model)
    
    # Extract model-like patterns
    # Pattern 1: Model names with version numbers
    version_pattern = r'\b([A-Z][a-zA-Z]+(?:[A-Z][a-zA-Z]*)*(?:-)?v?\d+(?:\.\d+)?)\b'
    matches = re.findall(version_pattern, text)
    models.update(m for m in matches if len(m) > 3)
    
    # Pattern 2: Acronyms in parentheses
    acronym_pattern = r'\(([A-Z]{2,}[A-Za-z0-9\-]*)\)'
    matches = re.findall(acronym_pattern, text)
    models.update(m for m in matches if 2 < len(m) < 15)
    
    return models

def extract_institutions_from_text(text, authors):
    """Extract institutions from text and author affiliations"""
    institutions = set()
    
    # Check for known institutions
    for inst in KNOWN_INSTITUTIONS:
        if inst.lower() in text.lower():
            institutions.add(inst)
    
    # Extract from email domains if present
    email_pattern = r'@([a-zA-Z0-9\-]+\.[a-zA-Z]{2,})'
    domains = re.findall(email_pattern, text)
    for domain in domains:
        if 'edu' in domain or 'ac.' in domain:
            inst_name = domain.split('.')[0].title()
            if len(inst_name) > 2:
                institutions.add(inst_name)
    
    return institutions

def categorize_paper(title, abstract, categories):
    """Categorize paper into high-level topics"""
    text = (title + " " + abstract).lower()
    
    topic_keywords = {
        "language-models": ["language model", "llm", "gpt", "bert", "transformer", "nlp", "text generation"],
        "computer-vision": ["vision", "image", "visual", "detection", "segmentation", "cv", "cnn"],
        "reinforcement-learning": ["reinforcement learning", "rl", "agent", "reward", "policy", "q-learning"],
        "generative-ai": ["generative", "diffusion", "gan", "vae", "synthesis", "generation"],
        "multimodal": ["multimodal", "cross-modal", "vision-language", "clip", "dalle"],
        "optimization": ["optimization", "efficient", "compression", "quantization", "pruning"],
        "robotics": ["robot", "robotic", "manipulation", "navigation", "embodied"],
        "security": ["security", "privacy", "adversarial", "attack", "defense", "safety"],
        "theory": ["theory", "theoretical", "proof", "bound", "convergence", "complexity"],
        "applications": ["application", "medical", "finance", "science", "engineering", "biology"]
    }
    
    topics = []
    for topic, keywords in topic_keywords.items():
        if any(keyword in text for keyword in keywords):
            topics.append(topic)
    
    return topics

def enhanced_extraction(papers):
    """Enhanced extraction with better categorization"""
    entities = []
    events = []
    
    # Statistics tracking
    author_stats = Counter()
    model_stats = Counter()
    institution_stats = Counter()
    topic_stats = Counter()
    yearly_stats = Counter()
    
    # Process each paper
    for i, paper in enumerate(papers):
        payload = paper.get("payload", {})
        
        title = payload.get("title", "")
        authors = payload.get("authors", [])
        categories = payload.get("categories", [])
        abstract = payload.get("abstract", "")
        url = payload.get("url", "")
        paper_id = paper.get("id", "")
        
        year = extract_year_from_url(url)
        if year:
            yearly_stats[year] += 1
        
        # Extract models
        full_text = title + " " + abstract
        models = extract_models_from_text(full_text)
        for model in models:
            model_stats[model] += 1
        
        # Extract institutions
        institutions = extract_institutions_from_text(full_text, authors)
        for inst in institutions:
            institution_stats[inst] += 1
        
        # Categorize paper
        topics = categorize_paper(title, abstract, categories)
        for topic in topics:
            topic_stats[topic] += 1
        
        # Track authors
        for author in authors:
            if author:
                author_stats[author] += 1
        
        # Create paper publication event
        if title and year:
            event = {
                "id": f"{year}-{str(paper_id)[:8]}",
                "timestamp": f"{year}-01-01T00:00:00Z",
                "description": f"{title}",
                "category": topics[0] if topics else "research"
            }
            events.append(event)
    
    # Create entities for prominent authors
    for author, count in author_stats.most_common(200):  # Top 200 authors
        if count >= 2:  # Authors with multiple papers
            entity = {
                "name": author,
                "summary": f"Prolific researcher with {count} papers in machine learning and AI. Active in cutting-edge research.",
                "tags": ["researcher", "ai-researcher", "prolific-author"]
            }
            entities.append(entity)
    
    # Create entities for frequently mentioned models
    for model, count in model_stats.most_common(100):
        if count >= 3:  # Models mentioned in multiple papers
            entity = {
                "name": model,
                "summary": f"Important AI model/architecture referenced in {count} research papers. Key component in modern machine learning.",
                "tags": ["ai-model", "architecture", "machine-learning"]
            }
            entities.append(entity)
    
    # Create entities for institutions
    for inst, count in institution_stats.most_common(50):
        if count >= 2:
            entity = {
                "name": inst,
                "summary": f"Leading research institution with {count} papers in the collection. Major contributor to AI/ML advancement.",
                "tags": ["institution", "research-lab", "university"]
            }
            entities.append(entity)
    
    # Create milestone events for research trends
    for topic, count in topic_stats.most_common():
        if count >= 20:
            event = {
                "id": f"2024-trend-{topic}",
                "timestamp": "2024-06-01T00:00:00Z",
                "description": f"Research trend: {count} papers in {topic.replace('-', ' ')} showing significant academic focus",
                "category": "research-trend"
            }
            events.append(event)
    
    # Create yearly milestone events
    for year, count in yearly_stats.most_common():
        if count >= 50:
            event = {
                "id": f"{year}-research-milestone",
                "timestamp": f"{year}-06-01T00:00:00Z",
                "description": f"Research milestone: {count} significant AI/ML papers published in {year}",
                "category": "yearly-milestone"
            }
            events.append(event)
    
    # Add some high-impact conceptual entities
    concepts = [
        {
            "name": "Transformer Architecture",
            "summary": "Revolutionary neural network architecture that powers modern language models and beyond. Introduced attention mechanism.",
            "tags": ["architecture", "deep-learning", "breakthrough", "attention"]
        },
        {
            "name": "Diffusion Models",
            "summary": "Generative modeling approach that creates high-quality images and other content through iterative denoising.",
            "tags": ["generative-ai", "image-generation", "probabilistic-models"]
        },
        {
            "name": "Large Language Models",
            "summary": "Massive neural networks trained on vast text corpora, capable of understanding and generating human-like text.",
            "tags": ["nlp", "ai-models", "foundation-models", "text-generation"]
        },
        {
            "name": "Foundation Models",
            "summary": "Large-scale models trained on broad data that can be adapted to many downstream tasks.",
            "tags": ["ai-paradigm", "transfer-learning", "general-ai"]
        },
        {
            "name": "Model Context Protocol",
            "summary": "Open standard by Anthropic for integrating AI models with external tools and data sources.",
            "tags": ["ai-integration", "protocol", "tool-use", "anthropic"]
        }
    ]
    entities.extend(concepts)
    
    return entities, events

def main():
    print("Extracting papers from Qdrant...")
    papers = extract_all_papers()
    print(f"Found {len(papers)} papers")
    
    print("Running enhanced extraction...")
    new_entities, new_events = enhanced_extraction(papers)
    
    # Load existing data
    with open("data/entities.json", "r") as f:
        entities = json.load(f)
    
    with open("data/events.json", "r") as f:
        events = json.load(f)
    
    # Merge avoiding duplicates
    existing_names = {e["name"] for e in entities}
    existing_ids = {e["id"] for e in events}
    
    # Add new unique entities
    for entity in new_entities:
        if entity["name"] not in existing_names:
            entities.append(entity)
            existing_names.add(entity["name"])
    
    # Add new unique events
    for event in new_events:
        if event["id"] not in existing_ids:
            events.append(event)
            existing_ids.add(event["id"])
    
    # Sort for consistency
    entities.sort(key=lambda x: x["name"])
    events.sort(key=lambda x: x["timestamp"], reverse=True)
    
    # Save
    with open("data/entities.json", "w") as f:
        json.dump(entities, f, indent=2)
    
    with open("data/events.json", "w") as f:
        json.dump(events, f, indent=2)
    
    print(f"Total entities: {len(entities)}")
    print(f"Total events: {len(events)}")
    print("Enhanced extraction complete!")

if __name__ == "__main__":
    main()