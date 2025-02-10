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
