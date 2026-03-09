# Setting Up Patent Search with Ata

## Overview

Ata can search patents worldwide via the [European Patent Office (EPO) Open Patent Services](https://www.epo.org/en/searching-for-patents/data/web-services/ops). The EPO database covers 90+ patent offices with daily updates, returning structured metadata including titles, abstracts, inventors, assignees, classification codes, and claims text.

## Prerequisites

Patent search requires EPO API credentials. Paper search and other research tools work without this setup.

## Getting EPO Credentials

1. Go to [https://developers.epo.org](https://developers.epo.org) and create an account
2. Register a new application to obtain a **Consumer Key** and **Consumer Secret**

## Configuration

Set the following environment variables:

| Variable | Required | Description |
| -------- | -------- | ----------- |
| `EPO_CONSUMER_KEY` | Yes | Your EPO OAuth2 consumer key |
| `EPO_CONSUMER_SECRET` | Yes | Your EPO OAuth2 consumer secret |

```shell
export EPO_CONSUMER_KEY="your-consumer-key"
export EPO_CONSUMER_SECRET="your-consumer-secret"
```

Both variables must be set for patent tools to be enabled. Ata handles OAuth2 token management (acquisition, caching, and refresh) automatically.

## Tools Available To Agents

| Tool | Description |
| ---- | ----------- |
| `patent_search` | Search patents by keyword, inventor, assignee, CPC code, and date range |
| `patent_get` | Get full patent details including claims text |

## Usage with Ata

Once configured, you can ask Ata to search patents naturally:

- "Find patents related to battery thermal management"
- "Search patents by inventor John Smith filed after 2020"
- "Get details for patent EP1234567"
