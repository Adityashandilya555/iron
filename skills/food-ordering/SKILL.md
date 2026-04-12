---
name: food-ordering
version: "0.2.0"
description: >
  Orchestrates food delivery ordering across Swiggy and Zomato platforms.
  Use this skill whenever the user wants to order food, search for restaurants,
  browse menus, add items to cart, apply coupons, place orders, or track deliveries.
  Triggers on: "order food", "I'm hungry", restaurant names, cuisine names (biryani,
  pizza, dosa, etc.), "what should I eat", "order from swiggy/zomato", menu browsing,
  cart operations, food delivery tracking. Also triggers when user mentions specific
  dishes, asks for recommendations, or says anything food-related. This skill handles
  the COMPLETE food ordering lifecycle from search to delivery tracking.
activation:
  keywords:
    - food
    - hungry
    - order
    - swiggy
    - zomato
    - restaurant
    - menu
    - biryani
    - pizza
    - burger
    - lunch
    - dinner
    - breakfast
    - delivery
    - coupon
    - cart
    - checkout
  patterns:
    - "(?i)(want|feel like|craving|looking for).{0,30}(eat|food|order|lunch|dinner)"
    - "(?i)(find|show|search).{0,30}(restaurant|place to eat|food)"
    - "(?i)I('m| am) hungry"
  tags:
    - food
    - delivery
    - ordering
  max_context_tokens: 2500
---

# Food Ordering — Swiggy + Zomato

Dual-platform food delivery. Query both, merge results, user picks platform per order.

## 1. Auth Check

Call `swiggy_auth(action="status")` and `zomato_auth(action="status")`.

- **Both valid:** Proceed.
- **One valid:** Search that platform. Auth the other using phone from `USER.md`.
- **Neither valid:** Read `workspace://USER.md → phone`. Call `swiggy_auth(action="start_auth", phone=<number>)`. Ask user for OTP only. After Swiggy, do Zomato same way.
- **Never re-ask phone number.** OTP only.

## 2. Address Resolution

Call both in parallel on first food request:
- `swiggy:get_addresses()` → use `id` as `addressId`
- `zomato:get_saved_addresses_for_user()` → use `address_id`

Match to user's residence from USER.md. Cache the pair for the session.

## 3. Search (parallel)

- `swiggy:search_restaurants(addressId, query)`
- `zomato:get_restaurants_for_keyword(address_id, keyword)`

If one fails, use the other.

**Result processing:**
1. Filter: Swiggy `availabilityStatus == "OPEN"` / Zomato `serviceability_status == "serviceable"`
2. Deduplicate by restaurant name + locality
3. Apply USER.md preferences: `diet` filter, `budget` filter, `cuisines` boost
4. Show top 5 with platform badges (🟠 Swiggy / 🔴 Zomato / 🟠🔴 Both)
5. Each result: name, rating, distance, ETA, cost for two, best offer

For vague queries ("I'm hungry"), use USER.md `cuisines` as the search term.

## 4. Menu Browse

Platform locked by restaurant selection.

**Swiggy:** `get_restaurant_menu(addressId, restaurantId)` — paginated categories.
For item details with variants/addons: `search_menu(addressId, query, restaurantIdOfAddedItem)`.

**Zomato:** `get_menu_items_listing(res_id, address_id)` → discover categories.
Then `get_restaurant_menu_by_categories(res_id, categories, address_id)` for full variant/addon data.

Display: filter by diet preference, highlight bestsellers, group by category, max 8 items per message.

## 5. Item Selection

**Always ask variant before adding to cart.** Show sizes/combos with prices.
Show available addons after variant is chosen.

Swiggy: items use either `variations` (legacy) or `variantsV2` — use matching field in cart.
Zomato: use `variant_id` (v_*), NOT `catalogue_id` (ctl_*). Addon choices are ctl_* IDs.

## 6. Cross-Platform Comparison

**Before cart creation**, if restaurant exists on both platforms:

1. Fetch pricing from both (Swiggy `search_menu` + `fetch_food_coupons` / Zomato menu data)
2. Show side-by-side: item price + delivery + best coupon + ETA per platform
3. User picks platform or says "cheapest" → lock platform for this order

Skip if restaurant is only on one platform.

## 6.5. Group Order (Optional)

After user picks platform but before creating cart, offer group ordering:
"Want to order with friends? I can set up a group order."

- **Solo** (default) → continue to §7 (Cart)
- **With friends** → hand off to `group-order` skill with context: restaurant name, platform, and address IDs

Skip this prompt if:
- User already indicated solo intent ("just me", "solo", "no")
- USER.md `friends[]` is empty and user hasn't mentioned friends
- User is in a hurry ("quick order", "just get me...")

## 7. Cart

**Swiggy:** `update_food_cart(restaurantId, cartItems, addressId)` then `get_food_cart(addressId)`.
Cart is session-local (not visible in user's app until order placed).

**Zomato:** `create_cart(res_id, items, address_id, payment_type)`. Always ask payment_type — `upi_qr` or `pay_later`.

Ask "Want to add anything else?" before proceeding.

## 8. Coupons

**Swiggy:** `fetch_food_coupons(restaurantId, addressId)` → show best applicable.
Apply with `apply_food_coupon(couponCode, addressId)`.
If adding ₹30-100 more unlocks a discount, mention once.

**Zomato:** `get_cart_offers(cart_id, address_id)` → apply via `promo_code` in `create_cart`.

## 9. Order Placement

**Mandatory before placing:** Show cart summary, delivery address, payment method, ETA. Get explicit confirmation.

**Swiggy:** `place_food_order(addressId, paymentMethod)`. Currently COD only — show only `availablePaymentMethods`. Cart limit ₹1000 (beta).

**Zomato:** `checkout_cart(cart_id)`. If `upi_qr`, display the returned QR code.

## 10. Post-Order

Confirm order ID + ETA. Offer tracking:
- Swiggy: `track_food_order(orderId)`
- Zomato: `get_order_tracking_info()`

**Cancellation:** No cancel API. Guide user to app → active order → Help → Cancel.

## Error Recovery

| Error | Action |
|-------|--------|
| Auth expired | Re-auth with USER.md phone, ask OTP only, resume flow |
| Platform unreachable | Fall back to other platform |
| Both platforms down | Tell user, suggest retry later |
| Restaurant closed | Suggest open alternatives from same search |
| Item out of stock | Show same-category alternatives |
| Cart >₹1000 (Swiggy) | Suggest Zomato or split order |
