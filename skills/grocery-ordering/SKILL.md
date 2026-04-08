---
name: grocery-ordering
version: 0.1.0
description: >
  Orchestrates grocery delivery via Swiggy Instamart. Use this skill whenever
  the user wants to order groceries, household items, snacks, beverages, dairy,
  fruits, vegetables, or any Instamart product. Triggers on: "order groceries",
  "need milk", "buy eggs", "instamart", "vegetables", "snacks", or any request
  for household/daily essentials. Handles the complete grocery lifecycle: auth
  check → product search (or quick-picks) → variant selection → cart build →
  review → checkout → optional tracking.
activation:
  keywords:
    - groceries
    - grocery
    - instamart
    - milk
    - eggs
    - vegetables
    - fruits
    - bread
    - butter
    - rice
    - dal
    - snacks
    - household
    - essentials
    - daily
    - supermarket
    - shopping
    - chips
    - biscuits
    - noodles
    - maggi
    - curd
    - paneer
    - onion
    - tomato
    - cooking
  patterns:
    - "(?i)(order|buy|get|need).{0,40}(groceries|grocery|instamart|vegetables|milk|eggs|snacks)"
    - "(?i)(running out of|out of|need more|low on).{0,30}\\w+"
    - "(?i)(weekly|monthly|daily).{0,20}(shopping|groceries|essentials)"
  tags:
    - grocery
    - instamart
    - swiggy
    - ordering
  max_context_tokens: 2500
---

# Grocery Ordering — Swiggy Instamart

Single-platform grocery delivery. Instamart only — no cross-platform comparison.

## 1. Auth Check

Call `swiggy_auth(action="status")`.

- **Valid:** Proceed to step 2.
- **Invalid/expired:**
  1. Read phone from `workspace://USER.md → phone`
  2. Call `swiggy_auth(action="start_auth", phone=<number>)` — silently, no user prompt
  3. Ask user **only for the OTP code** ("Enter the OTP Swiggy just sent you!")
  4. Call `swiggy_auth(action="verify_otp", otp=<user_input>)`
  5. Resume from the interrupted step

**Never ask for phone number.** OTP only. Phone is captured in onboarding Q2 and lives in USER.md forever.

## 2. Address Resolution

Call `swiggy-instamart:get_addresses()`.

- Match the returned addresses to `USER.md → residence` by locality/area name.
- Use the matched `id` as `addressId` / `selectedAddressId` for all subsequent calls.
- Cache it for the session — don't call get_addresses again unless the user asks to change delivery location.
- If no address matches: show the returned addresses and ask user to pick one.

## 3. Smart Start

### If user specified items (e.g., "need milk and eggs"):
- Call `swiggy-instamart:search_products(addressId, query)` for each item in parallel.
- Group results by item for clarity.

### If user said something vague ("groceries", "stock up", "what do you have"):
- Call `swiggy-instamart:your_go_to_items(addressId)` first.
- Show as quick-picks: "Here's what you usually order — want to add any of these?"
- If user wants something specific, then run `search_products`.

## 4. Showing Product Results

For each product:
- Show name, pack size/weight, price
- If `isPromoted: true` → label as "📢 Sponsored"
- **Always show variants** — never assume which size the user wants

Format (max 5 per search):
```
🥛 Milk options:
1. Amul Taaza (500ml) — ₹28
2. Amul Taaza (1L) — ₹54
3. Nandini Toned Milk (1L) — ₹52
   📢 Sponsored

Which size? Or type a number.
```

**Always ask which variant before adding to cart.** "Amul milk" is not enough — get the pack size confirmed.

## 5. Cart Management

**Critical:** `update_cart` REPLACES the entire cart. Track all cart items locally across the conversation.

When user selects a variant:
1. Add to local cart state: `{ itemId, storeId, quantity, name, price }`
2. Call `swiggy-instamart:update_cart(selectedAddressId, items=[...all accumulated items...])`
3. Confirm: "Added Amul Taaza 1L! Anything else?"
4. **Warn once** (on first item added): "Just so you know — your cart here is temporary until you checkout. It won't show up in the Swiggy app until you place the order."

When user wants to remove an item:
1. Remove from local state
2. Call `update_cart` with the updated items array
3. If user removes everything: call `clear_cart(addressId)` instead

When user wants to change quantity:
1. Update quantity in local state
2. Call `update_cart` with full updated array

**Multi-store warning:** If cart items come from multiple Instamart stores, warn:
```
Heads up — your cart has items from 2 stores, so these'll be 2 separate orders
with 2 delivery fees. Want to continue, or should I narrow it to one store?
```

## 6. Cart Review

When user says "done", "that's it", "checkout", or has been adding items for a while:

Call `swiggy-instamart:get_cart(addressId)`.

Show full breakdown:
```
Here's your cart 🛒

🥛 Amul Taaza 1L × 2 — ₹108
🥚 Farm Fresh Eggs (12 pcs) — ₹89
🍞 Britannia Brown Bread — ₹35
─────────────────────────
Subtotal: ₹232
Delivery: ₹25
Total: ₹257

📍 Delivering to: Home — HSR Layout, Bangalore
💳 Payment: Cash on Delivery

Place order?
```

**Always show delivery address and payment method before asking for confirmation.** No surprises.

## 7. Checkout

After explicit user confirmation ("yes", "place it", "go ahead"):

Call `swiggy-instamart:checkout(addressId, paymentMethod)`.

- Payment method: COD (Cash on Delivery) is available on Instamart. Use `availablePaymentMethods` from `get_cart` to show all options — COD will be present.
- On success: show order ID + estimated delivery time
- Order now syncs to the user's Swiggy app (account-level, unlike the session-local cart)

```
Order placed! 🎉

Your groceries are on the way.
Order #IM-XXXXX | ETA: ~15 min

Want me to track the delivery?
```

On checkout error:
- Check if the error mentions out-of-stock items → remove them, ask user if they want to proceed without
- Check if it's an address issue → show get_addresses and ask to re-pick
- For other errors: show the error message, suggest retry

## 8. Tracking (Optional)

If user asks to track after checkout:

Call `swiggy-instamart:track_order(orderId, lat, lng)`.
- Get lat/lng from the address object returned by `get_addresses`
- Show delivery partner status, name, and ETA
- Offer to check again in a few minutes if user wants updates

## 9. Past Orders (Optional)

If user asks "what did I order last time" or "my order history":

Call `swiggy-instamart:get_orders()`.
- Show last 3-5 orders with date, items, and total
- Offer to re-order from a past order (pre-fill cart with those items)

## Error Recovery

| Error | Action |
|-------|--------|
| Auth expired mid-flow | Re-auth silently: phone from USER.md, ask OTP only, resume |
| Product not found | Suggest alternative search terms; offer go-to items instead |
| Item out of stock | Remove from cart, tell user, suggest similar products |
| Multi-store cart | Warn user about separate orders + double delivery fee |
| Cart state lost | Call `get_cart` to recover from server, re-sync local state |
| Checkout fails | Show reason, offer to fix (remove OOS items, change address) |
| Address not found | Show all addresses from get_addresses, ask user to pick |

## Cancellation

No cancel API exists for Instamart orders.

```
To cancel this order:
1. Open the Swiggy app → go to your active order
2. Tap 'Help' → 'I want to cancel'
3. Follow the steps there

Do it quickly — Instamart orders are packed fast!
```

## Personality Notes

- Grocery shopping is routine — be efficient, not chatty
- But spot a deal or a missing item: "You got milk but no bread? Classic breakfast setup waiting to happen 😄"
- For hostel students: know that bulk packs save money — mention it once if relevant
- "Sponsored" items are okay to show, just label them honestly
- Don't push upsells. If user has what they need, get to checkout.
