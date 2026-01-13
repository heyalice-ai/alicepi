import logging
import requests
import io
from typing import Callable, List, Dict
from pydub import AudioSegment
from .base import BaseEngine
from src import config

logger = logging.getLogger("Orchestrator.CloudEngine")

class CloudEngine(BaseEngine):
    def __init__(self):
        self.api_url = config.CLOUD_API_URL
        self.voice_id = config.CLOUD_VOICE_ID
        self.tenant_id = config.CLOUD_TENANT_ID

    def process(self, text: str, history: List[Dict[str, str]], on_audio_chunk: Callable[[bytes], None]) -> str:
        """
        Calls the cloud API with the user's text.
        The API returns an MP3 stream.
        Decodes the MP3 stream and yields PCM chunks.
        """
        payload = {
            "query": text,
            "voiceId": self.voice_id,
        }
        if self.tenant_id:
            payload["tenantId"] = self.tenant_id
        
        # Note: In a real implementation, we might want to send history as well 
        # if the cloud API supports it (e.g. via conversationId).
        # For now, we follow the VOICE_SERVICE_API.md spec.

        logger.info(f"Calling Cloud API: {self.api_url} for text: {text[:50]}...")
        
        try:
            response = requests.post(
                self.api_url,
                json=payload,
                headers={"Accept": "audio/mpeg"},
                stream=True,
                timeout=30
            )
            response.raise_for_status()

            # The cloud API sends MP3. We need to decode it to PCM.
            # Since MP3 is not easily chunk-decodable without a streaming decoder,
            # we might need to buffer it or use a library that supports streaming.
            # pydub is better for whole files, but we can feed it chunks if they are valid frames.
            
            # Simple approach for now: collect the whole response or large chunks.
            # Real-time streaming of MP3 to PCM is slightly more complex.
            
            full_audio_data = b""
            for chunk in response.iter_content(chunk_size=8192):
                if chunk:
                    full_audio_data += chunk
            
            if full_audio_data:
                # Decode MP3 to PCM
                audio = AudioSegment.from_file(io.BytesIO(full_audio_data), format="mp3")
                
                # Convert to raw PCM bytes
                # We'll pass the raw data to the callback. 
                # The callback should be wrapped with the AudioProcessor to ensure final format.
                
                # pydub gives us pcm_data via audio.raw_data
                # We info the processor about the source format:
                # audio.frame_rate, audio.channels, audio.sample_width (itemsize)
                
                # We can stream it in chunks to the callback to simulate streaming
                chunk_size = 4096
                raw_data = audio.raw_data
                for i in range(0, len(raw_data), chunk_size):
                    on_audio_chunk(
                        raw_data[i:i+chunk_size], 
                        audio.frame_rate, 
                        audio.channels, 
                        f"int{audio.sample_width * 8}"
                    )
                
                # Return a placeholder or mock response text
                # In a real cloud flow, the response text might also be returned in headers or a separate field.
                # Assuming the cloud API just returns audio for now.
                return "The cloud response has been played."

        except Exception as e:
            logger.error(f"Cloud API call failed: {e}")
            return f"Error: Cloud service unavailable. {e}"

        return ""
