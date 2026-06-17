---
name: odoo-readonly
description: Use for Odoo CRM lookups — helpdesk tickets, invoices, partners/contacts. All writes are forbidden.
version: 1
risk: low
allowed_tools:
  - odoo_search_contacts
  - odoo_get_invoices
  - odoo_list_tickets
  - odoo_get_record
tags:
  - odoo
  - crm
  - customer
  - partner
  - invoice
  - helpdesk
---

# Odoo Read-Only Skill

Use this skill when the user asks about Odoo data: helpdesk tickets, invoices, contacts, or partners.

## Capabilities

- **Helpdesk tickets** — list tickets assigned to the user (`helpdesk.ticket`), filter open/closed.
- **Invoices** — list customer and vendor invoices (`account.move`), filter by paid/unpaid status.
- **Contacts / Partners** — search `res.partner` by name, email, or phone.
- **Open in Safari** — emit an `odoo_found` event so the UI can show an "Otvoriť v Safari" button.

All data is fetched live from the configured Odoo 18 instance. Never fabricate Odoo data.

## Constraints

- **All writes are forbidden.** Do not create, update, delete records, or send email from Odoo.
- If the connector is not configured (no credentials), inform the user and direct them to Settings → Odoo.
- If an API call fails, say so clearly. Do not invent data to fill gaps.

## Slovak Odoo terminology — preserve verbatim

Slovak business terms in Odoo data must never be translated or paraphrased:
- **IČO** (identification number), **DIČ** (tax ID), **DPH** (VAT)
- **zákazník** (customer), **partner**, **objednávka** (order)
- **faktúra** (invoice), **tiket** (ticket)

Present all monetary amounts with the currency as returned by Odoo (e.g. `1 200,50 EUR`).
