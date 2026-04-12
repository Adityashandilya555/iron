---
name: table-booking
version: 0.1.0
description: >
  Orchestrates restaurant table reservations via Swiggy Dineout. Use this skill
  whenever the user wants to book a table, make a reservation, dine out, find
  restaurants for dine-in, check available slots, or plan a dinner outing.
  Triggers on: "book a table", "dineout", "reservation", "dinner out", "want
  to sit and eat", "dine-in", restaurant names with booking intent. Handles the
  complete booking lifecycle: auth → location → search → details → slots →
  confirmation → book → verify status.
activation:
  keywords:
    - table
    - booking
    - dineout
    - reservation
    - dine
    - dinner
    - brunch
    - lunch out
    - eat out
    - restaurant
    - seated
    - reserve
  patterns:
    - "(?i)(book|reserve|find).{0,30}(table|restaurant|place to eat|dine)"
    - "(?i)(want to|let's|planning to).{0,30}(eat out|dine out|go out for|sit and eat)"
    - "(?i)(dinner|brunch|lunch).{0,20}(tonight|tomorrow|this weekend|on \\w+day)"
  tags:
    - dineout
    - booking
    - table
    - swiggy
  max_context_tokens: 2500
---

# Table Booking — Swiggy Dineout

Single-platform table reservations. Dineout only — bookings sync to Swiggy app immediately.

## 1. Auth Check

Call `swiggy_auth(action="status")`.

- **Valid:** Proceed.
- **Invalid/expired:**
  1. Read phone from `workspace://USER.md → phone`
  2. Call `swiggy_auth(action="start_auth", phone=<number>)` silently
  3. Ask user **only for the OTP code**
  4. Call `swiggy_auth(action="verify_otp", otp=<user_input>)`
  5. Resume

**Never ask for phone number.** OTP only.

## 2. Location Resolution

Call `swiggy-dineout:get_saved_locations()`.

- Match to `USER.md → residence` by locality/area name.
- Extract `lat` and `lng` from the matched location.
- **These coordinates are STICKY** — save them and pass to every subsequent Dineout call in this session.
- If no saved location matches: use the user's known area (e.g., "Koramangala") as the search query and extract coordinates from the search results.

## 3. Search Restaurants

### If user named a specific restaurant:
- Call `swiggy-dineout:search_restaurants_dineout(query, addressId)` OR use `lat`+`lng` directly.
- If exact match found, skip to step 4.
- If multiple matches, show top 3 and ask user to pick.

### If user said something vague ("dinner tonight", "somewhere nice"):
- Ask ONE question: "What kind of cuisine are you in the mood for? Or any area you prefer?"
- Then search with that input.

**Result presentation (max 5):**
```
🍽 Dinner spots near you:

1. Toit Brewpub
   ⭐ 4.5 • Indiranagar • Multi-cuisine
   ₹1500 for two • Known for craft beer

2. Truffles
   ⭐ 4.3 • Koramangala • Burgers & Shakes
   ₹800 for two • Always has a queue

3. Vidyarthi Bhavan
   ⭐ 4.7 • Basavanagudi • South Indian
   ₹200 for two • Legendary masala dosa

Which one? Or search for something else.
```

**Save lat/lng from the search response** — these become the sticky coordinates.

## 4. Restaurant Details

After user picks a restaurant:

Call `swiggy-dineout:get_restaurant_details(restaurantId, lat, lng)`.

Show:
- Cuisines, locality, rating
- Timings (opening/closing hours)
- Key amenities (parking, wifi, outdoor seating, etc.)
- Cost for two
- Menu images if available (send as Telegram photo message or link)

**Check timings BEFORE proceeding.** If the restaurant is currently closed or closes soon:
```
"Toit is closed right now (opens at 12 PM). Want to book for later today, or try somewhere else?"
```

Then ask the booking details — **one question at a time:**
1. "When? (date)" — default to today if user said "tonight"
2. "What time?"
3. "How many people?"

If the user already provided all three ("tomorrow 8pm, 4 people"), skip ahead.

## 5. Available Slots

Call `swiggy-dineout:get_available_slots(restaurantId, date, lat, lng, guestCount)`.

**Critical:** Only show slots where `isFree == true`. Paid deals are not supported and will fail at booking.

**If no free slots:**
```
"No free deals available at Toit for April 8. Want to:
- Try a different date?
- Check other restaurants nearby?
- Book without a deal? (just the table)"
```

**If free slots exist:**
```
Available slots at Toit for 4 guests on April 8:

1. 7:30 PM — 20% off total bill (Free!)
2. 8:00 PM — 15% off total bill (Free!)
3. 8:30 PM — 20% off total bill (Free!)
4. 9:00 PM — 10% off food bill (Free!)

Pick a time?
```

Show the deal/offer for each slot — this is the main value prop of Dineout.

## 6. Confirmation (MANDATORY)

**Always confirm before booking.** Bookings are account-level and sync immediately to the Swiggy app.

```
Confirming your reservation:

🍽 Toit Brewpub, Indiranagar
📅 Tuesday, April 8
🕗 8:00 PM
👥 4 guests
🏷 15% off total bill (Free deal)

Book this table?
```

Wait for explicit confirmation ("yes", "book it", "confirm").

## 7. Book Table

After user confirms:

Call `swiggy-dineout:book_table(restaurantId, slotId, itemId, reservationTime, guestCount, lat, lng)`.

**Critical type conversion:** `reservationTime` is returned as a STRING in the slots response, but `book_table` expects it as a NUMBER. Parse the string to a number before passing it.

**On success:**
```
Table booked! 🎉

🍽 Toit Brewpub, Indiranagar
📅 April 8 at 8:00 PM • 4 guests
🏷 15% off total bill

✅ This booking is already in your Swiggy app — show it at the restaurant.
Booking ID: #XXXXX
```

**On failure:**
- Slot taken: "That slot just got booked! Here are the remaining ones: [re-fetch slots]"
- Auth issue: silently re-auth, retry once
- Other error: show the error, suggest trying a different slot or restaurant

## 8. Verify Booking

After `book_table` succeeds:

Call `swiggy-dineout:get_booking_status(orderId)`.

- Confirm status is `COMPLETED` (or equivalent success status).
- If pending/processing: wait a moment and check again (one retry).
- If failed: tell user and suggest rebooking.

## Error Recovery

| Error | Action |
|-------|--------|
| Auth expired | Re-auth: phone from USER.md, ask OTP only, resume |
| Restaurant closed | Check timings, suggest booking for when it opens |
| No free deals | Offer different date, different restaurant, or no-deal booking |
| Slot just taken | Re-fetch slots, show remaining options |
| Guest count too large | Suggest splitting group or calling restaurant directly |
| Booking failed | Show error, offer to retry with different slot |
| Location not found | Fall back to coordinate-based search using known area |

## Cancellation / Modification

No cancel API exists for Dineout bookings through MCP.

```
To cancel or modify this booking:
1. Open the Swiggy app → go to your Dineout bookings
2. Find this reservation → tap 'Manage Booking'
3. Follow the steps to cancel or change

Tip: Cancel early — some restaurants charge for no-shows!
```

## Personality Notes

- Dineout is the "going out" vibe — slightly more excited tone than grocery ordering
- Know Bangalore spots: Toit, Truffles, MTR, Vidyarthi Bhavan, Windmills, CTR, Permit Room
- For college students: highlight the free deals — "20% off just for booking through here, not bad right?"
- Don't oversell. If user has decided, get to the booking fast.
- Group bookings are common (college friend groups) — handle large party sizes gracefully
- Weekend dinner + birthday = very common use case — if user mentions birthday, note that some restaurants do free cake/decoration (check amenities)
