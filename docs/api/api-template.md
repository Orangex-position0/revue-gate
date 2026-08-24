# API Documentation

## Basics

- Service name: <service-name>
- API version: v<major>.<minor>.<patch>
- Base URL: <protocol>://<domain>:<port>/<base-path>
- Authentication: <Bearer Token / Cookie Session / API Key / None>
- Data format: request `<content-type>`, response `<content-type>`

## Endpoint List

| Method | Path                 | Name            | Auth  | Description |
| ------ | -------------------- | --------------- | ----- | ----------- |
| GET    | `/api/v1/<resource>` | <endpoint name> | <yes/no> | <purpose> |

## Endpoint Details

### <METHOD> <PATH> · <Endpoint name>

Description: <what this endpoint does>

Request URL: `<protocol>://<domain>:<port>/<path>`

Request headers:

| Name          | Type   | Required | Example          | Notes |
| ------------- | ------ | -------- | ---------------- | ----- |
| Authorization | string | yes      | `Bearer <token>` | <auth notes> |

Request parameters:

| Name        | Location        | Type   | Required | Default | Range   | Format   | Example   | Notes |
| ----------- | --------------- | ------ | -------- | ------- | ------- | -------- | --------- | ----- |
| <paramName> | path/query/body | string | yes      | <none>  | <range> | <format> | <example> | <notes> |

Request example:

```bash
curl -X <METHOD> '<url>' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer <token>' \
  -d '<json-body>'
```

Response fields:

| Name        | Type   | Required | Format   | Range   | Example   | Notes |
| ----------- | ------ | -------- | -------- | ------- | --------- | ----- |
| <fieldName> | string | yes      | <format> | <range> | <example> | <notes> |

Response example:

```json
{
    "data": {}
}
```

Error codes:

| HTTP Status | Code             | Description        | Suggested handling |
| ----------- | ---------------- | ------------------ | ------------------ |
| 400         | validation_error | Invalid parameters | Check parameter format and allowed values |

Security notes:

- Authorization: <describe authentication and authorization>
- Transport encryption: <state whether HTTPS is required>
- Sensitive data: <list sensitive fields and masking requirements>

## Versioning

- Current version: v<major>.<minor>.<patch>
- Compatibility policy: <compatibility and deprecation policy>

## Change Log

| Version | Date       | Author | Summary |
| ------- | ---------- | ------ | ------- |
| v0.1.0 | YYYY-MM-DD | <name> | Initialize API documentation |
