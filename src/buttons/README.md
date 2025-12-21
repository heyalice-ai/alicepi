# AlicePi Buttons Service

Manages physical user interactions via GPIO on Raspberry Pi.

## GPIO Mapping
- **GPIO 17**: RESET (Short press) / FACTORY\_RESET (Long press)
- **GPIO 27**: VOLUME\_UP
- **GPIO 22**: VOLUME\_DOWN

## Protocols
- **Output**: ZMQ PUB on port 5558. Messages are JSON strings: `{"event": "EVENT_NAME", "timestamp": "ISO_TIMESTAMP"}`.
