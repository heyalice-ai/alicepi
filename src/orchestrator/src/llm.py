import logging
import requests
import json
from . import config

logger = logging.getLogger("Orchestrator.LLM")

class LLMClient:
    def __init__(self, api_url=None, system_prompt=None):
        self.api_url = api_url or config.LLM_API_URL
        self.system_prompt = system_prompt or config.SYSTEM_PROMPT

    def call(self, history):
        """Call the configured LLM API using chat completions format."""
        try:
            messages = [{"role": "system", "content": self.system_prompt}] + history
            
            payload = {
                "model": "llama3", 
                "messages": messages,
                "stream": False
            }
            
            logger.info(f"Calling LLM: {self.api_url}")
            response = requests.post(self.api_url, json=payload, timeout=15)
            response.raise_for_status()
            
            data = response.json()
            if "choices" in data and len(data["choices"]) > 0:
                return data["choices"][0]["message"]["content"]
            elif "message" in data: 
                return data["message"]["content"]
                
            logger.error(f"Unexpected LLM response format: {data}")
            return "I'm sorry, I couldn't process that response."
            
        except Exception as e:
            logger.error(f"Error calling LLM: {e}")
            return "I'm having trouble connecting to my brain right now."
