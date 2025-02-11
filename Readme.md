# One Million Sliders

This project provides an API to manage a large number of sliders (1,000,000). The API allows you to set slider values, retrieve snapshots of slider states, and subscribe to updates.

## Base URL

The base URL for the API is: `https://onemillionsliders.com`

## API Endpoints

### 1. Get Range Snapshot

**Endpoint:** `GET /snapshot`

**Description:** Retrieves a snapshot of the slider states within a specified range.

**Query Parameters:**
- `start` (u64): The starting index of the range. This value will be rounded down to the nearest multiple of 1024.
- `end` (u64): The ending index of the range. This value will be rounded up to the nearest multiple of 1024.

**Response:**
- `200 OK`: Returns a JSON object containing the start index and the base64-encoded bits representing the slider states.
- `400 Bad Request`: If the range is invalid.

**Example Request:**
```http
GET /snapshot?start=1&end=100
```

Note that this will return bits from index 0 to 1023.

**Example Response:**
```json
{
  "start": 0,
  "bits": "base64-encoded-bits"
}
```

### 2. Get Range Updates

**Endpoint:** `GET /updates`

**Description:** Subscribes to updates for the slider states within a specified range.

**Query Parameters:**
- `start` (u64): The starting index of the range. This value will be rounded down to the nearest multiple of 1024.
- `end` (u64): The ending index of the range. This value will be rounded up to the nearest multiple of 1024.

**Response:**
- `200 OK`: Returns an SSE (Server-Sent Events) stream of updates.
- `400 Bad Request`: If the range is invalid.

**Example Request:**
```http
GET /updates?start=0&end=100
```

**Server-Sent Event Types:**
- `update`: Sent when a chunk of slider states within the specified range is updated. The `data` field contains the base64-encoded bits representing the updated slider states. The `id` field represents the chunk index the update is for (chunks are 1024 bits).
- `sum`: Sent periodically with the sum of all slider values. The `data` field contains the sum.

**Example Response:**
```http
event: update
data: base64-encoded-bits
id: 0

event: sum
data: 12345
```

### 3. Set Slider Value

**Endpoint:** `POST /set_byte/:idx/:value`

**Description:** Sets the value of a slider at the specified index.

**Path Parameters:**
- `idx` (u64): The index of the slider to set.
- `value` (u8): The value to set the slider to (in range 0-255)

**Response:**
- `200 OK`: If the slider value was successfully set.
- `400 Bad Request`: If the index is out of range.

**Example Request:**
```http
POST /set_byte/100/255
```

**Example Response:**
```http
200 OK
```

### 4. Get State Image

**Endpoint:** `GET /image.png`

**Description:** Returns a 1000x1000 grayscale PNG image representation of all slider values. Each pixel's brightness corresponds to a slider value (0-255).

**Response:**
- `200 OK`: Returns a PNG image

**Example Request:**
```http
GET /image.png
```

### 5. WebSocket Batch Updates

**Endpoint:** `GET /ws`

**Description:** Opens a WebSocket connection for sending batch updates to multiple sliders efficiently.

**Message Types:**

1. Individual Updates (0x00)
   - First byte: 0x00
   - Subsequent bytes: List of operations (max 1), where each operation is 5 bytes:
     - Bytes 2-5: Slider index (32-bit little-endian unsigned integer)
     - Byte 6: Value to set (0-255)

   **Example:**
   ```hex
   00 05 000000
   ```
   This message:
   - Uses message type 0x00
   - Sets slider #5 to 255

2. Block Update (0x01)
   - First byte: 0x01
   - Bytes 2-5: Starting index (32-bit little-endian unsigned integer)
   - Subsequent bytes: Values to set (max 1 byte)

   **Example:**
   ```hex
   01 05 000000 FF
   ```
   This message:
   - Uses message type 0x01
   - Sets slider #5 to 255

Note: All multi-byte integers are encoded in little-endian format.

**Limitations:**
- Maximum 1 operations per message for type 0x00
- Maximum 1 values per message for type 0x01
- Slider indices must be within range (0 to 999,999)

**Error Handling:**
- The WebSocket connection will be closed with an appropriate error message if:
  - Message type is invalid
  - Message format is invalid
  - Operation/value limit is exceeded
  - Any slider index is out of range
