# Sherpa REST API Documentation

## Base URL

```
https://<server-ip>:3030
```

## Authentication

The Sherpa API uses JWT (JSON Web Tokens) for authentication. Most endpoints require a valid JWT token in the `Authorization` header.

### Getting a Token

**Endpoint:** `POST /api/v1/auth/login`

**Request:**
```json
{
  "username": "alice",
  "password": "SecurePass123!"
}
```

**Response (200 OK):**
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "username": "alice",
  "is_admin": false,
  "expires_at": 1234567890
}
```

**Errors:**
- `401 Unauthorized` - Invalid username or password
- `500 Internal Server Error` - Server error

**Example (curl):**
```bash
curl -X POST https://server:3030/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"SecurePass123!"}'
```

### Using the Token

Include the token in the `Authorization` header for all protected endpoints:

```
Authorization: Bearer <your-token-here>
```

**Example:**
```bash
curl https://server:3030/api/v1/labs/my-lab \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
```

### Token Expiration

Tokens expire after 7 days (604800 seconds). You'll receive a `401 Unauthorized` error if your token has expired. Simply login again to get a new token.

---

## Endpoints

### Health Check

Check if the server is running.

**Endpoint:** `GET /health`

**Authentication:** None required

**Response (200 OK):**
```json
{
  "status": "ok",
  "service": "sherpad",
  "tls": "enabled"
}
```

**Example:**
```bash
curl https://server:3030/health
```

---

### Login

Authenticate and receive a JWT token.

**Endpoint:** `POST /api/v1/auth/login`

**Authentication:** None required

**Request Body:**
```json
{
  "username": "string",
  "password": "string"
}
```

**Response (200 OK):**
```json
{
  "token": "string (JWT)",
  "username": "string",
  "is_admin": boolean,
  "expires_at": number (Unix timestamp)
}
```

**Errors:**
- `401 Unauthorized` - Invalid credentials
- `500 Internal Server Error` - Server error

**Example:**
```bash
curl -X POST https://server:3030/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "alice",
    "password": "SecurePass123!"
  }'
```

---

### Get Lab Details

Get detailed information about a specific lab.

**Endpoint:** `GET /api/v1/labs/{id}`

**Authentication:** Required (JWT token)

**Path Parameters:**
- `id` (string) - The lab ID

**Response (200 OK):**
```json
{
  "lab_info": {
    "id": "my-lab",
    "name": "My Network Lab",
    "description": "A test lab",
    "topology": { ... }
  },
  "devices": [
    {
      "name": "router1",
      "model": "VEOS",
      "kind": "VM",
      "active": true,
      "mgmt_ip": "192.168.100.10",
      "disks": [
        "router1-my-lab-disk1.qcow2"
      ]
    }
  ],
  "inactive_devices": [
    "router2"
  ]
}
```

**Response Fields:**

- `lab_info` - Lab metadata and configuration
  - `id` - Unique lab identifier
  - `name` - Human-readable lab name
  - `description` - Lab description
  - `topology` - Network topology configuration
  
- `devices` - Array of active devices with:
  - `name` - Device name
  - `model` - Device model (e.g., VEOS, SROS, SR Linux)
  - `kind` - Device type: `VM`, `Container`, or `Unikernel`
  - `active` - Whether device is currently running
  - `mgmt_ip` - Management IP address (DHCP-assigned)
  - `disks` - Array of disk volume names
  
- `inactive_devices` - Array of device names that should exist but aren't running

**Authorization:**
- Users can only inspect their own labs
- Admin users can inspect any lab

**Errors:**
- `401 Unauthorized` - Missing or invalid token
- `403 Forbidden` - User doesn't own this lab
- `404 Not Found` - Lab doesn't exist
- `500 Internal Server Error` - Server error

**Example:**
```bash
# Save token for reuse
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."

# Get lab details
curl https://server:3030/api/v1/labs/my-lab \
  -H "Authorization: Bearer $TOKEN"
```

---

## Error Response Format

All errors return a consistent JSON structure:

```json
{
  "error": {
    "code": "ERROR_CODE",
    "message": "Human-readable error message",
    "details": "Additional context (optional)"
  }
}
```

### Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `UNAUTHORIZED` | 401 | Missing or invalid authentication |
| `FORBIDDEN` | 403 | Valid auth but insufficient permissions |
| `NOT_FOUND` | 404 | Requested resource doesn't exist |
| `BAD_REQUEST` | 400 | Invalid request format or parameters |
| `INTERNAL_ERROR` | 500 | Server-side error |

**Example Error Response:**
```json
{
  "error": {
    "code": "FORBIDDEN",
    "message": "Access denied",
    "details": "Permission denied: Lab 'my-lab' is owned by another user"
  }
}
```

---

## CORS

The API supports Cross-Origin Resource Sharing (CORS) to allow web applications to access the API from different domains.

**Allowed:**
- All origins (configurable for production)
- All HTTP methods (GET, POST, DELETE, etc.)
- All headers
- Credentials (cookies, auth headers)

**Note:** For production deployments, configure specific allowed origins in the server configuration.

---

## TLS/HTTPS

The Sherpa server uses TLS for secure connections. You can download the server certificate:

**Endpoint:** `GET /cert`

**Note:** This endpoint is available over HTTP (port 3031) to allow clients to fetch the certificate before trusting it.

**Example:**
```bash
# Download server certificate
curl http://server:3031/cert > server.crt

# Install certificate (system-dependent)
# Linux:
sudo cp server.crt /usr/local/share/ca-certificates/
sudo update-ca-certificates

# macOS:
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain server.crt
```

---

## Rate Limiting

Rate limiting is not currently implemented.

---

## Testing the API

### Basic Workflow

1. **Check server health:**
```bash
curl -k https://server:3030/health
```

2. **Login to get token:**
```bash
TOKEN=$(curl -k -X POST https://server:3030/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"YourPassword"}' \
  | jq -r '.token')

echo "Token: $TOKEN"
```

3. **Inspect a lab:**
```bash
curl -k https://server:3030/api/v1/labs/my-lab \
  -H "Authorization: Bearer $TOKEN" \
  | jq
```

### Testing Authentication Errors

**Test missing token:**
```bash
curl -k https://server:3030/api/v1/labs/my-lab
# Expected: 401 Unauthorized
```

**Test invalid token:**
```bash
curl -k https://server:3030/api/v1/labs/my-lab \
  -H "Authorization: Bearer invalid-token"
# Expected: 401 Unauthorized
```

**Test expired token:**
```bash
# Use a token that's older than 7 days
curl -k https://server:3030/api/v1/labs/my-lab \
  -H "Authorization: Bearer <expired-token>"
# Expected: 401 Unauthorized
```

**Test accessing another user's lab:**
```bash
# Login as user A
TOKEN_A=$(curl -k -X POST https://server:3030/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"..."}' \
  | jq -r '.token')

# Try to access user B's lab
curl -k https://server:3030/api/v1/labs/bobs-lab \
  -H "Authorization: Bearer $TOKEN_A"
# Expected: 403 Forbidden
```

---

## Version History

### v1 (Current)
- Initial API release
- Authentication via JWT
- Lab inspection endpoint
- Health check endpoint
- CORS support
- Structured error responses

---

## Future Endpoints (Planned)

The following endpoints are planned for future releases:

- `GET /api/v1/labs` - List all user's labs
- `POST /api/v1/labs` - Create a new lab
- `DELETE /api/v1/labs/{id}` - Destroy a lab
- `POST /api/v1/labs/{id}/start` - Start a lab
- `POST /api/v1/labs/{id}/stop` - Stop a lab

---

## Support

For issues or questions, please refer to the project documentation or open an issue on the project repository.
