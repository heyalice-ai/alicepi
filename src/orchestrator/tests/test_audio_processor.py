import numpy as np
from src.audio_processor import AudioProcessor

def test_remuxer_mono_to_stereo():
    processor = AudioProcessor(target_rate=48000, target_channels=2, target_dtype='int32')
    
    # Mono, 16kHz, 16-bit
    src_rate = 16000
    src_channels = 1
    src_dtype = 'int16'
    
    # Create 1 second of audio
    t = np.linspace(0, 1, src_rate, endpoint=False)
    src_data = (np.sin(2 * np.pi * 440 * t) * 32767).astype(np.int16)
    src_bytes = src_data.tobytes()
    
    processed_bytes = processor.process_chunk(src_bytes, src_rate, src_channels, src_dtype)
    
    # Check shape/size
    # Target: 48000 samples, 2 channels, 32-bit (4 bytes)
    expected_size = 48000 * 2 * 4
    print(f"Expected: {expected_size}, Got: {len(processed_bytes)}")
    # Allow 4 frames (32 bytes) of difference due to audioop behavior
    assert abs(len(processed_bytes) - expected_size) <= 32
    
    # Check content
    processed_data = np.frombuffer(processed_bytes, dtype=np.int32).reshape(-1, 2)
    assert abs(processed_data.shape[0] - 48000) <= 4
    # Check that channels are identical (since it was mono)
    assert np.all(processed_data[:, 0] == processed_data[:, 1])

def test_remuxer_scaling():
    processor = AudioProcessor(target_rate=16000, target_channels=1, target_dtype='int32')
    
    # 16-bit to 32-bit scaling
    src_data = np.array([32767, -32768, 0], dtype=np.int16)
    src_bytes = src_data.tobytes()
    
    processed_bytes = processor.process_chunk(src_bytes, 16000, 1, 'int16')
    processed_data = np.frombuffer(processed_bytes, dtype=np.int32)
    
    # 32767 << 16 = 2147418112
    assert processed_data[0] == 32767 << 16
    assert processed_data[1] == -32768 << 16
    assert processed_data[2] == 0

if __name__ == "__main__":
    print("Running tests...")
    test_remuxer_mono_to_stereo()
    print("test_remuxer_mono_to_stereo passed!")
    test_remuxer_scaling()
    print("test_remuxer_scaling passed!")
    print("All tests passed!")
