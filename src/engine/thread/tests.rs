use super::*;
use crate::engine::html;
use serde_json::json;

fn chat() -> Thread {
    Thread::starter(Kind::Chat, "Dark chat")
}

fn forum() -> Thread {
    Thread::starter(Kind::Forum, "Classic forum")
}

#[test]
fn markup_covers_the_light_syntax() {
    assert_eq!(markup("**bold** and *it* and `code`"), "<b>bold</b> and <i>it</i> and <code>code</code>");
    assert_eq!(markup("a < b & \"c\""), "a &lt; b &amp; &quot;c&quot;");
    assert_eq!(markup("||secret||"), "<span class=\"sp\">secret</span>");
    assert_eq!(markup("~~old~~ __under__"), "<s>old</s> <u>under</u>");
    assert_eq!(markup("hi @Jordan see #general"), "hi <span class=\"mn\">@Jordan</span> see <span class=\"mn\">#general</span>");
    assert_eq!(markup("a#1 and user@mail"), "a#1 and user@mail", "not mentions inside a word");
    assert_eq!(markup(":fire: and 😀 and :nope:"), "🔥 and 😀 and :nope:");
    assert_eq!(markup("> quoted\nnot quoted"), "<blockquote>quoted</blockquote>not quoted");
    assert_eq!(markup("```\nlet x = 1;\n<b>\n```"), "<pre><code>let x = 1;\n&lt;b&gt;</code></pre>");
    assert_eq!(markup("```rust\nfn a() {}\n```after"), "<pre><code>fn a() {}</code></pre>after");
    assert_eq!(markup("1 * 2 * 3 and snake_case_name"), "1 * 2 * 3 and snake_case_name");
    assert_eq!(markup("**a *b* c**"), "<b>a <i>b</i> c</b>");
    // a link keeps its text but never a network URL in the page (the library rules)
    let l = markup("see https://example.com/x?a=1, ok");
    assert!(l.contains("lk") && !l.contains("https://") && l.contains("example.com/x?a=1</span>,"), "{l}");
    assert_eq!(markup("line one\nline two"), "line one<br>line two");
    assert_eq!(markup("**unclosed"), "**unclosed");
}

#[test]
fn counts_read_like_a_forum() {
    for (n, t) in [(0, "0"), (950, "950"), (1000, "1k"), (1234, "1.2k"), (12400, "12.4k"), (999_999, "999.9k"), (1_100_000, "1.1m"), (-5, "-5")] {
        assert_eq!(count_text(n), t, "{n}");
    }
    assert_eq!(count_of("4.2k"), Some(4200));
    assert_eq!(count_of("+3"), Some(3));
    assert_eq!(count_of("12 pts"), Some(12));
    assert_eq!(count_of("3h"), None);
}

#[test]
fn timestamps_follow_the_format_and_now() {
    let mut t = chat();
    let it = Item { date: "2026-11-13".into(), time: "16:32".into(), ..Default::default() };
    assert_eq!(t.stamp(&it), "Today at 4:32 PM", "now is 2026-11-13");
    let y = Item { date: "2026-11-12".into(), time: "4:32 PM".into(), ..Default::default() };
    assert_eq!(t.stamp(&y), "Yesterday at 4:32 PM");
    let old = Item { date: "2026-10-01".into(), time: "9:05 am".into(), ..Default::default() };
    assert_eq!(t.stamp(&old), "10/1/2026 9:05 AM");
    for (f, want) in [("time", "4:32 PM"), ("24h", "16:32"), ("full", "11/13/2026 4:32 PM"), ("relative", "8 minutes ago")] {
        t.time_format = f.into();
        assert_eq!(t.stamp(&it), want, "{f}");
    }
    t.time_format = "relative".into();
    assert_eq!(t.stamp(&old), "43 days ago");
    assert_eq!(parse_date("11/13/26"), Some((2026, 11, 13)));
    assert_eq!(parse_time("4pm"), Some((16, 0)));
    assert_eq!(parse_time("12:05 AM"), Some((0, 5)));
    assert_eq!(parse_time("25:00"), None);
    assert_eq!(civil(days((2026, 12, 31)) + 1), (2027, 1, 1));
}

#[test]
fn a_new_message_follows_the_one_before_it_in_time() {
    let mut t = chat();
    let id = t.add(Item::msg("Maya", "hi"), None);
    let it = &t.items[t.index_of(id).unwrap()];
    assert_eq!((it.date.as_str(), it.time.as_str()), ("2026-11-13", "4:33 PM"));
    let mut t = Thread::blank(Kind::Chat, "Dark chat");
    t.items.push(Item { id: 1, who: "A".into(), date: "2026-11-13".into(), time: "11:59 PM".into(), ..Default::default() });
    let id = t.add(Item::msg("A", "late"), None);
    let it = &t.items[t.index_of(id).unwrap()];
    assert_eq!((it.date.as_str(), it.time.as_str()), ("2026-11-14", "12:00 AM"), "rolls over midnight");
}

#[test]
fn consecutive_messages_of_one_person_group() {
    let mut t = Thread::blank(Kind::Chat, "Dark chat");
    for (who, time) in [("Ann", "4:30 PM"), ("Ann", "4:31 PM"), ("Bob", "4:31 PM"), ("Ann", "4:32 PM"), ("Ann", "4:50 PM")] {
        t.items.push(Item { id: t.items.len() as u32 + 1, who: who.into(), text: "x".into(), time: time.into(), ..Default::default() });
    }
    let page = t.html();
    // headers: Ann 4:30, Bob, Ann 4:32 and Ann 4:50 (more than 7 minutes later) - the 4:31 one is grouped
    assert_eq!(page.matches("class=\"hd\"").count(), 4, "{page}");
    assert_eq!(page.matches("class=\"gt\"").count(), 1);
    t.group = false;
    assert_eq!(t.html().matches("class=\"hd\"").count(), 5);
    t.group = true;
    t.items[1].reply = Some(1);
    assert_eq!(t.html().matches("class=\"hd\"").count(), 5, "a reply keeps its own header");
    t.layout = "compact".into();
    assert_eq!(t.html().matches("class=\"hd\"").count(), 0, "compact has one line per message");
}

#[test]
fn the_list_edits_keep_ids_order_and_replies_straight() {
    let mut t = chat();
    let ids: Vec<u32> = t.items.iter().map(|i| i.id).collect();
    assert_eq!(ids, [1, 2, 3]);
    let d = t.duplicate(2).unwrap();
    assert_eq!(t.items.iter().map(|i| i.id).collect::<Vec<_>>(), [1, 2, d, 3]);
    assert!(t.move_by(d, -2) && t.items[0].id == d);
    assert!(t.move_to(d, 99) && t.items.last().unwrap().id == d, "clamped to the end");
    assert!(!t.move_by(777, 1) && t.duplicate(777).is_none());
    t.items[2].reply = Some(1);
    assert!(t.remove(1));
    assert_eq!(t.items.iter().find(|i| i.id == 3).unwrap().reply, None, "a reply to a removed message is dropped");
    let n = t.add(Item::msg("Zed", "late"), Some(0));
    assert_eq!(t.items[0].id, n, "inserted at the front");
    assert!(t.member("zed").is_some(), "a new speaker joins the cast");
    assert_eq!(t.items.iter().map(|i| i.id).collect::<std::collections::HashSet<_>>().len(), t.items.len(), "ids stay unique");
}

#[test]
fn changing_a_cast_member_changes_all_their_messages() {
    let mut t = chat();
    let before = t.html();
    t.upsert(Member { name: "maya".into(), color: "#112233".into(), badge: "BOT".into(), ..Default::default() });
    assert_eq!(t.cast.len(), 2, "same name (any case) updates, never duplicates");
    let page = t.html();
    assert_eq!(page.matches("color:#112233").count(), 2, "both of Maya's messages follow");
    assert!(page.contains("class=\"bdg\">BOT") && !before.contains("bdg\">BOT"));
    assert!(t.rename_member("Maya", "Maya R"));
    assert!(t.items.iter().filter(|i| i.who == "Maya R").count() == 2 && t.member("Maya").is_none());
    assert!(!t.rename_member("Maya R", "Jordan"), "no two people of one name");
    assert!(t.remove_member("jordan") && t.member("Jordan").is_none());
    assert!(t.html().contains("Jordan"), "their messages stay, with a coloured circle");
}

#[test]
fn a_chat_script_makes_messages() {
    let mut t = Thread::blank(Kind::Chat, "Dark chat");
    let n = t.from_script(
        "alice: hello there\nbob (4:32 PM): hi!\ncarol (11/13/2026 4:35 PM): third **bold**\n  continued line\n--- Today\n* dave joined the server\n!unread\nbob: last\\nwith break",
        true,
    );
    assert_eq!(n, 7);
    let it = &t.items;
    assert_eq!((it[0].who.as_str(), it[0].text.as_str()), ("alice", "hello there"));
    assert_eq!(it[1].time, "4:32 PM");
    assert_eq!((it[2].date.as_str(), it[2].time.as_str()), ("11/13/2026", "4:35 PM"));
    assert_eq!(it[2].text, "third **bold**\ncontinued line", "an unmatched line continues the message");
    assert_eq!((it[3].kind, it[3].text.as_str()), (ItemKind::Divider, "Today"));
    assert_eq!((it[4].kind, it[4].who.as_str(), it[4].text.as_str()), (ItemKind::System, "dave", "joined the server"));
    assert_eq!(it[5].kind, ItemKind::Unread);
    assert_eq!(it[6].text, "last\nwith break");
    assert_eq!(t.cast.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["alice", "bob", "carol", "dave"], "speakers join the cast");
    // a URL or a time in the text is not a speaker
    let mut t = Thread::blank(Kind::Chat, "Dark chat");
    t.from_script("x: look at https://a.b/c\nmeet at 4:30 sharp", true);
    assert_eq!(t.items.len(), 1);
    assert!(t.items[0].text.ends_with("meet at 4:30 sharp"));
    // append keeps what is there
    assert_eq!(t.from_script("y: more", false), 1);
    assert_eq!(t.items.len(), 2);
    assert_eq!(t.items[1].time, "", "no clock to follow: left empty");
}

#[test]
fn a_forum_script_nests_by_marks_and_indent() {
    let mut t = Thread::blank(Kind::Forum, "Classic forum");
    t.from_script(
        "# The title\n## First line\\nsecond\nann (3h, 4.2k): top\n> bob (2h, 12): reply\n>> cy: deeper\n  dee: indented reply\n    eve: more indent\nfay: top again",
        true,
    );
    assert_eq!((t.post.title.as_str(), t.post.body.as_str()), ("The title", "First line\nsecond"));
    let d: Vec<(&str, u8)> = t.items.iter().map(|i| (i.who.as_str(), i.depth)).collect();
    assert_eq!(d, [("ann", 0), ("bob", 1), ("cy", 2), ("dee", 1), ("eve", 2), ("fay", 0)]);
    assert_eq!((t.items[0].age.as_str(), t.items[0].score), ("3h", 4200));
    assert_eq!(t.items[1].score, 12);
    // a depth can only go one deeper than the comment before it
    t.items[5].depth = 7;
    t.normalize();
    assert_eq!(t.items[5].depth, 3);
    t.items[5].depth = 0;
}

#[test]
fn the_script_reads_back_what_it_wrote() {
    for mut t in [chat(), forum()] {
        let again_src = t.to_script();
        let mut u = Thread::blank(t.kind, "Dark chat");
        u.from_script(&again_src, true);
        assert_eq!(u.items.len(), t.items.len(), "{again_src}");
        for (a, b) in t.items.iter().zip(&u.items) {
            assert_eq!((&a.who, &a.text, a.depth, &a.time, &a.date, a.score), (&b.who, &b.text, b.depth, &b.time, &b.date, b.score));
        }
        t.items.clear();
    }
}

#[test]
fn the_thread_rides_in_the_page_and_comes_back() {
    for t in [chat(), forum()] {
        let page = t.html();
        let back = from_src(&page).expect("the page carries its thread");
        assert_eq!(back, t);
        assert!(from_src("<html><body>x</body></html>").is_none());
        let plain = detach(&page);
        assert!(from_src(&plain).is_none() && plain.contains("<style>") && !plain.contains("se:thread"));
    }
    // a thread full of awkward text survives the attribute and the JSON
    let mut t = Thread::blank(Kind::Chat, "Dark chat");
    t.add(Item::msg("A\"b", "quote \" amp & lt < gt > https://x.y/z \\ back"), None);
    assert_eq!(from_src(&t.html()).unwrap(), t);
}

#[test]
fn every_page_follows_the_library_rules() {
    for style in ["Dark chat", "Light chat", "Compact chat", "Classic forum", "Card forum", "Dark forum", "Light comments", "Dark comments", "Compact comments"] {
        let kind = preset(style).unwrap().1;
        let mut t = Thread::starter(kind, style);
        t.items.push(Item { who: "Q".into(), text: "link https://example.com 0:42 and `code` ||hid||".into(), images: vec!["pic.png".into()], embed: Some(Embed { title: "T".into(), desc: "d".into(), color: "#ff0000".into(), thumb: "t.png".into() }), reactions: vec![Reaction { emoji: "🔥".into(), count: 2, me: false }], pulse: Some(0.5), ..Default::default() });
        if kind == Kind::Chat {
            t.rail = true;
            t.sidebar = true;
        }
        let page = t.html();
        assert_eq!(html::lint(&page), Vec::<String>::new(), "{style}");
        let tpl = html::parse(&page);
        assert_eq!(tpl.size, (t.width, t.height));
        assert_eq!(tpl.params.len(), 1, "the scroll param");
        assert!(page.contains("class=\"win"), "{style}");
        t.anim.on = false;
        assert_eq!(html::lint(&t.html()), Vec::<String>::new(), "{style} without animation");
    }
    for (name, mode) in [("chat", ""), ("chat_typing", "dm"), ("chat_typing", "group"), ("chat_typing", "server"), ("forum", ""), ("comments", ""), ("comments_typing", "")] {
        let t = Thread::recipe(name, mode, "").unwrap();
        assert_eq!(html::lint(&t.html()), Vec::<String>::new(), "{name} {mode}");
        assert_eq!(from_src(&t.html()).unwrap(), t, "{name} {mode} round-trips");
    }
}

#[test]
fn every_chat_toggle_adds_and_removes_only_its_own_piece() {
    // (a toggle, a marker in the page that only that piece has)
    type Set = fn(&mut Thread, bool);
    let toggles: [(&str, Set, &str); 10] = [
        ("rail", |t, v| t.rail = v, "class=\"rail\""),
        ("channels", |t, v| t.channels = v, "class=\"chs\""),
        ("sidebar", |t, v| t.sidebar = v, "class=\"side\""),
        ("topbar", |t, v| t.topbar = v, "class=\"tb\""),
        ("header", |t, v| t.header = v, "class=\"hd0"),
        ("composer", |t, v| t.composer = v, "class=\"cmp\""),
        ("indicator", |t, v| t.indicator = v, "class=\"tyw\""),
        ("dividers", |t, v| t.dividers = v, "class=\"dv\""),
        ("unread_line", |t, v| t.unread_line = v, "class=\"dv nw\""),
        ("scrollbar", |t, v| t.scrollbar = v, "class=\"sbar\""),
    ];
    let mut base = chat();
    base.items.push(Item { kind: ItemKind::Divider, text: "Today".into(), ..Default::default() });
    base.items.push(Item { kind: ItemKind::Unread, ..Default::default() });
    let body = |t: &Thread| t.html().split("</head>").nth(1).unwrap().to_string();
    base.set_messages_only();
    assert!(base.messages_only());
    let off = body(&base);
    assert!(!off.contains("class=\"rail\"") && !off.contains("class=\"hd0") && !off.contains("class=\"foot\"") && !off.contains("class=\"dv"), "messages only is just the messages: {off}");
    assert!(off.contains("Maya") && off.contains("it was"), "the messages are there");
    for (name, set, marker) in toggles {
        let mut t = base.clone();
        set(&mut t, true);
        assert!(!t.messages_only(), "{name}");
        let on = body(&t);
        assert!(on.contains(marker), "{name} on shows {marker}");
        for (other, _, m2) in toggles {
            if other != name && m2 != marker && !marker.starts_with(m2) && !m2.starts_with(marker) {
                assert!(!on.contains(m2) || other == "composer" && name == "indicator", "{name} alone must not show {other}'s {m2}");
            }
        }
        set(&mut t, false);
        assert!(t.messages_only() && body(&t) == off, "{name} off again is messages only");
        assert_eq!(html::lint(&t.html()), Vec::<String>::new());
    }
    // every combination of the ten still renders a page the library accepts (spot-check 64 of 1024)
    for bits in (0..1024u32).step_by(16) {
        let mut t = base.clone();
        for (k, (_, set, _)) in toggles.iter().enumerate() {
            set(&mut t, bits >> k & 1 == 1);
        }
        for mode in ["server", "dm", "group"] {
            t.mode = mode.into();
            t.transparent = bits % 2 == 0;
            assert_eq!(html::lint(&t.html()), Vec::<String>::new(), "{bits:b} {mode}");
        }
    }
    // transparent: no window background
    let mut t = base.clone();
    assert!(!body(&t).contains(" tr\""));
    t.transparent = true;
    assert!(body(&t).contains(" tr\""));
}

#[test]
fn a_dm_header_names_the_other_person_and_a_group_lists_the_rest() {
    let mut t = Thread::chat_typing("dm", "");
    let page = t.html();
    assert!(page.contains("class=\"hd0 dmh\"") && page.contains("<b>Jordan</b>") && page.contains("Online"), "{page}");
    assert!(page.contains("Message @Jordan"));
    t.mode = "group".into();
    t.title.clear();
    t.upsert(Member { name: "Dave".into(), ..Default::default() });
    let g = t.html();
    assert!(g.contains("<b>Jordan, Dave</b>") && g.contains("class=\"stack\""), "{g}");
    t.mode = "server".into();
    assert!(t.html().contains("class=\"hash\">#"));
    t.channels = true;
    assert!(t.html().contains("TEXT CHANNELS"));
    t.mode = "dm".into();
    assert!(t.html().contains("DIRECT MESSAGES"));
}

#[test]
fn typing_is_a_plan_of_keys_with_exact_times() {
    let (keys, total) = type_plan("hello", 10.0, false);
    assert_eq!(keys.len(), 5);
    assert!((keys[3].at - 0.3).abs() < 1e-9 && (total - 0.5).abs() < 1e-9);
    assert_eq!(typed_at(&keys, -0.1), "");
    assert_eq!(typed_at(&keys, 0.0), "h");
    assert_eq!(typed_at(&keys, 0.25), "hel");
    assert_eq!(typed_at(&keys, 9.0), "hello");
    // typos: deterministic, the wrong key is there for a moment and then gone, the text ends the same
    let text = "this is a rather long message with plenty of letters to mistype";
    let (a, ta) = type_plan(text, 12.0, true);
    let (b, tb) = type_plan(text, 12.0, true);
    assert_eq!((&a, ta), (&b, tb), "the same text makes the same typos");
    let wrong: Vec<&Key> = a.iter().filter(|k| k.until.is_some()).collect();
    assert!(!wrong.is_empty(), "this text gets a typo");
    assert!(ta > text.chars().count() as f64 / 12.0, "typos take longer");
    for w in &wrong {
        let mid = (w.at + w.until.unwrap()) / 2.0;
        assert!(typed_at(&a, mid).contains(w.ch.as_str()) || typed_at(&a, w.at).ends_with(w.ch.as_str()));
        assert!(!typed_at(&a, w.until.unwrap() + 1e-6).is_empty());
    }
    assert_eq!(typed_at(&a, ta + 1.0), text);
    assert_eq!(type_plan(text, 12.0, false).0.len(), text.chars().count());
    // an empty text and a crazy speed do not break it
    assert_eq!(type_plan("", 14.0, true).1, 0.0);
    assert!(type_plan("ab", 1e9, false).1 < 0.1);
}

#[test]
fn a_chat_types_what_the_person_at_the_screen_sends() {
    let t = Thread::chat_typing("dm", "");
    assert!(t.anim.compose && t.me == "Maya" && t.composer && t.indicator);
    let s = t.schedule();
    for (it, s) in t.items.iter().zip(&s) {
        assert_eq!(s.compose, it.who == "Maya", "{}: only `me` is typed into the box", it.who);
    }
    // each typed message: keys, then the pause, then it shows; the next one starts a gap later
    let (keys, total) = t.plan_of(&t.items[1]);
    assert_eq!(keys.len(), t.items[1].text.chars().count());
    assert!((s[1].appear - (s[1].type_start + total + t.anim.pause)).abs() < 1e-9);
    assert!((s[1].press - (s[1].type_start + total + t.anim.pause * 0.4)).abs() < 1e-9);
    assert!((s[2].type_start - (s[1].appear + t.anim.gap)).abs() < 1e-9);
    // the others show "is typing" for the typing time; the page has both
    assert!((s[0].typing - 0.9).abs() < 1e-9 && !s[0].compose);
    let page = t.html();
    assert_eq!(page.matches("class=\"tbox\"").count(), 2, "one overlay per typed message");
    assert_eq!(page.matches("is typing").count(), 2, "one line per message of the other person");
    assert!(page.contains("class=\"cr\"") && page.contains("class=\"c\""));
    // what is in the box at a moment = the keys so far, and nothing before or after
    let t1 = s[1].type_start;
    assert!(t1 > s[0].appear);
    // typing turned off: nobody types, the messages just come
    let mut u = t.clone();
    u.anim.compose = false;
    assert!(u.schedule().iter().all(|s| !s.compose));
    assert!(!u.html().contains("class=\"tbox\""));
    // a message box that is not there cannot be typed into
    let mut v = t.clone();
    v.composer = false;
    assert!(v.schedule().iter().all(|s| !s.compose));
    // per item: a speed of its own, a length of its own
    let mut w = t.clone();
    w.items[1].cps = Some(40.0);
    assert!(w.schedule()[1].typing < s[1].typing / 2.0);
    w.items[1].typing = Some(3.0);
    assert!((w.schedule()[1].typing - 3.0).abs() < 1e-6, "typing 3 s means 3 s");
    w.items[1].typed = Some(false);
    assert!(!w.schedule()[1].compose, "typed: no overrides the auto rule");
}

#[test]
fn video_comments_have_the_whole_section() {
    let t = Thread::starter(Kind::Comments, "Dark comments");
    let page = t.html();
    let body = page.split("</head>").nth(1).unwrap();
    for want in ["1,284 Comments", "Add a comment…", "Pinned by @MayaPlays", "class=\"hr\"", "2 replies", "(edited)", "creator", "class=\"lk\">0:42", "Reply", "12.4k"] {
        assert!(body.contains(want), "{want} in {body}");
    }
    assert_eq!(body.matches("class=\"av sm\"").count(), 3, "the reply avatars (and the heart's) are smaller: 2 replies and the heart");
    let mut c = t.clone();
    c.items[0].collapsed = true;
    let b = c.html().split("</head>").nth(1).unwrap().to_string();
    assert!(b.contains("▸ 2 replies") && !b.contains("Rewatched it 5 times"));
    let mut c = t.clone();
    c.layout = "compact".into();
    c.theme = "light".into();
    assert!(c.html().contains("c-compact") && c.html().contains("class=\"win fwin comments light"));
    // a comment can be at most one level deep
    c.items[2].depth = 5;
    c.normalize();
    assert!(c.items.iter().all(|i| i.depth <= 1));
    // the script reads comments like forum lines
    let mut s = Thread::blank(Kind::Comments, "");
    s.from_script("@a (3 days ago, 12K): first\n> @b (1 day ago, 5): reply\n@c: second", true);
    assert_eq!(s.items.iter().map(|i| (i.depth, i.score)).collect::<Vec<_>>(), [(0, 12000), (1, 5), (0, 1)]);
}

#[test]
fn a_typed_comment_opens_the_box_presses_the_button_and_posts_at_the_top() {
    let t = Thread::comments_typing("");
    let s = t.schedule();
    assert!(t.items[0].who == "@You" && s[0].compose && !s[0].shown, "the typed comment is first");
    assert!(t.items[2..].iter().all(|i| i.there), "the others were already there");
    assert!(s[1].appear > s[0].appear, "the creator's reply comes after the post: {s:?}");
    let (keys, total) = t.plan_of(&t.items[0]);
    assert!((s[0].appear - (s[0].type_start + total + t.anim.pause)).abs() < 1e-9);
    assert_eq!(typed_at(&keys, total + 0.01), t.items[0].text);
    let body = t.html().split("</head>").nth(1).unwrap().to_string();
    assert!(body.contains("class=\"cbtn\"") && body.contains("Comment</span>") && body.contains("class=\"cai\""));
    assert_eq!(body.matches("class=\"tbox\"").count(), 1);
    // the button is where the geometry says (em-based boxes), and the mouse goes there and presses it
    let ([x, y, w, h], press) = t.comment_button().unwrap();
    let em = t.size as f64;
    assert!((x + w - (t.width as f64 - em)).abs() < 1e-9 && (w - 7.0 * em).abs() < 1e-9 && (y + h / 2.0 - 7.9 * em).abs() < 1e-9);
    assert!((press - s[0].press).abs() < 1e-9 && press < s[0].appear && press > s[0].type_start);
    let m = t.mouse().unwrap();
    let (mx, my, down) = m.state(press + 0.05);
    assert!((mx - (x + w / 2.0)).abs() < 1.0 && (my - (y + h / 2.0)).abs() < 1.0 && down, "the mouse presses the button");
    assert!(!m.state(press - 0.2).2 && !m.state(press + 0.5).2, "pressed only for a moment");
    let (sx, _, _) = m.state(0.0);
    assert!(sx < x, "it starts away from the button");
    // a typed reply opens its own box under the comment, then closes
    let mut r = Thread::starter(Kind::Comments, "");
    for it in &mut r.items {
        it.there = true;
    }
    r.anim.compose = true;
    r.add(Item { who: "@You".into(), text: "me too".into(), depth: 1, typed: Some(true), ..Default::default() }, Some(1));
    let page = r.html().split("</head>").nth(1).unwrap().to_string();
    assert!(page.contains("class=\"rbx\"") && page.contains("Add a reply…") && page.contains(">Reply</span>"), "{page}");
    assert!(r.comment_button().is_none(), "a typed reply has no top Comment button");
    // deterministic: the same data, the same page, and every item is timed in the data (no script)
    assert_eq!(t.html(), t.html());
    assert!(!t.html().to_ascii_lowercase().contains("<script"));
}

#[test]
fn comments_count_with_commas_and_timestamps_link() {
    assert_eq!(comma(0), "0");
    assert_eq!(comma(1284), "1,284");
    assert_eq!(comma(1_234_567), "1,234,567");
    assert_eq!(comma(-12345), "-12,345");
    assert_eq!(markup_x("at 0:42 and 1:02:03 not 12:3 or 4:30pm", true), "at <span class=\"lk\">0:42</span> and <span class=\"lk\">1:02:03</span> not 12:3 or 4:30pm");
    assert_eq!(markup("at 0:42"), "at 0:42", "chat does not link times");
}

#[test]
fn the_schedule_is_exact_and_ordered() {
    let mut t = chat();
    t.anim = Anim { start: 1.0, gap: 2.0, typing: 0.0, ..Default::default() };
    let at: Vec<f64> = t.schedule().iter().map(|s| s.appear).collect();
    assert_eq!(at, [1.0, 3.0, 5.0]);
    t.items[1].delay = Some(0.5);
    assert_eq!(t.schedule().iter().map(|s| s.appear).collect::<Vec<_>>(), [1.0, 1.5, 3.5], "a per-item delay replaces the gap");
    t.anim.shown = 2;
    let s = t.schedule();
    assert!(s[0].shown && s[1].shown && !s[2].shown && s[2].appear == 1.0, "history is there from the start; the first new one waits the start");
    t.items[0].there = true;
    t.anim.shown = 0;
    assert!(t.schedule()[0].shown && !t.schedule()[1].shown, "one item can be there from the start wherever it is");
    t.items[0].there = false;
    t.items[1].delay = None;
    // the typing line: needs the toggle; an item starts typing `gap` after the one before shows
    t.anim.typing = 0.9;
    t.anim.start = 0.2;
    assert!(t.schedule().iter().all(|s| s.typing == 0.0), "messages only: no typing line, so no typing time");
    t.indicator = true;
    let s = t.schedule();
    assert_eq!((s[0].type_start, s[0].typing, s[0].appear), (0.2, 0.9, 1.1));
    assert!((s[1].type_start - 3.1).abs() < 1e-9 && (s[1].appear - 4.0).abs() < 1e-9, "typing starts one gap after the message before: {s:?}");
    t.items[1].typing = Some(0.0);
    assert_eq!(t.schedule()[1].typing, 0.0, "an item can turn its own indicator off");
    t.anim.on = false;
    assert!(t.schedule().iter().all(|s| s.shown));
    // the page: delays are inline, the typing line is in the footer, nothing needs script
    t.anim.on = true;
    t.items[1].typing = None;
    let page = t.html();
    assert!(page.contains("--d:1.1s") && page.contains("is typing"), "{page}");
    assert!(!page.to_ascii_lowercase().contains("<script"));
    assert_eq!(t.html(), t.html(), "the same data makes the same page");
}

#[test]
fn votes_tick_up_in_steps_and_replies_count() {
    let t = forum();
    let page = t.html();
    assert!(page.contains("class=\"tkc\" style=\"--n:12;--td:1.2s;--ts:0.3s\">1<br>"), "{page}");
    assert!(page.contains("12.4k</span>"), "it ends on the real count");
    assert!(page.contains("3 Comments"));
    let mut t = t;
    t.anim.tick = false;
    assert!(!t.html().contains("class=\"tkc\""));
    t.post.comments = 42;
    assert!(t.html().contains("42 Comments"));
    // collapsed comments hide their replies and do not take schedule time
    t.anim.tick = true;
    t.items[0].collapsed = true;
    let s = t.schedule();
    assert!(s[1].shown, "hidden under a collapsed comment");
    let page = t.html();
    let body = page.split("</head>").nth(1).unwrap();
    assert!(body.contains("[+] 1 hidden reply") && !body.contains("I regret everything"));
    t.items[0].collapsed = false;
    t.items[0].more = 5;
    assert!(t.html().contains("5 more replies"));
    assert_eq!(t.html().matches("class=\"ln\"").count(), 1, "one indent line for the one reply");
}

#[test]
fn a_patch_changes_what_it_names_and_refuses_what_it_does_not_know() {
    let mut t = chat();
    t.patch(&json!({"theme": "light", "anim": {"gap": 3.0}, "title": "random"})).unwrap();
    assert_eq!((t.theme.as_str(), t.anim.gap, t.title.as_str(), t.anim.start), ("light", 3.0, "random", 0.6));
    assert!(t.patch(&json!({"colour": "red"})).unwrap_err().contains("colour"));
    t.patch(&json!({"style": "Compact chat"})).unwrap();
    assert_eq!((t.layout.as_str(), t.theme.as_str()), ("compact", "dark"));
    assert!(t.patch(&json!({"style": "Card forum"})).unwrap_err().contains("unknown style"));
    assert!(t.patch(&json!({"kind": "forum"})).is_err());
    t.patch_item(1, &json!({"text": "changed", "reactions": [{"emoji": ":fire:", "count": 9}]})).unwrap();
    assert_eq!((t.items[0].text.as_str(), t.items[0].reactions[0].count, t.items[0].who.as_str()), ("changed", 9, "Maya"));
    assert!(t.patch_item(1, &json!({"nope": 1})).is_err() && t.patch_item(99, &json!({})).is_err());
    t.patch_item(1, &json!({"reactions": null})).unwrap();
    assert!(t.items[0].reactions.is_empty(), "null clears a field");
    // bad numbers cannot break the page
    t.patch(&json!({"width": 3, "size": 9999, "accent": "javascript"})).unwrap();
    assert!(t.width >= 320 && t.size <= 80 && t.html().contains("--accent:#5b6cf0"));
}

#[test]
fn presets_match_by_name_and_kind() {
    assert_eq!(preset("dark_chat").unwrap().0, "Dark chat");
    assert_eq!(preset("Card Forum").unwrap().3, "light");
    assert!(preset("pink").is_none());
    let t = Thread::starter(Kind::Forum, "Dark chat");
    assert_eq!((t.layout.as_str(), t.theme.as_str()), ("classic", "light"), "a style of the other kind falls back to the kind's first");
    assert_eq!(chat().preset_name(), Some("Dark chat"));
    let mut t = chat();
    t.accent = "#ff0000".into();
    assert!(t.apply_preset("light chat") && t.preset_name() == Some("Light chat") && t.accent == "#ff0000");
    assert!(!t.apply_preset("Dark forum"));
}

/// The two library templates are the starter threads: `SE_UPDATE_THREAD_LIBRARY=1 cargo test
/// library_templates` rewrites them (then `scripts/library-index.ps1`).
#[test]
fn library_templates_are_the_starters() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library/html");
    for (file, name, mode, style) in TEMPLATES {
        let want = Thread::recipe(name, mode, style).unwrap().html();
        let path = root.join(file);
        if std::env::var("SE_UPDATE_THREAD_LIBRARY").is_ok() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(have, want, "{file} is out of date: SE_UPDATE_THREAD_LIBRARY=1 cargo test library_templates");
    }
}

/// The Gallery cards: (file, recipe, chat layout, look).
const TEMPLATES: [(&str, &str, &str, &str); 6] = [
    ("chat-thread.html", "chat", "", "Dark chat"),
    ("chat-dm-typing.html", "chat_typing", "dm", "Dark chat"),
    ("chat-server-typing.html", "chat_typing", "server", "Dark chat"),
    ("forum-thread.html", "forum", "", "Classic forum"),
    ("video-comments.html", "comments", "", "Light comments"),
    ("video-comment-typing.html", "comments_typing", "", "Light comments"),
];

/// `SE_THREAD_DUMP=<dir> cargo test dump_prepared_pages -- --ignored`: the pages as Edge gets them (look at them).
#[test]
#[ignore]
fn dump_prepared_pages() {
    let Ok(dir) = std::env::var("SE_THREAD_DUMP") else { return };
    let at: f64 = std::env::var("SE_THREAD_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(99.0);
    let _ = at;
    let mut all: Vec<(String, Thread)> = Vec::new();
    for style in ["Dark chat", "Light chat", "Compact chat", "Classic forum", "Card forum", "Dark forum", "Light comments", "Dark comments", "Compact comments"] {
        let kind = preset(style).unwrap().1;
        let mut t = Thread::starter(kind, style);
        if kind == Kind::Chat {
            (t.rail, t.channels, t.header, t.composer, t.sidebar) = (true, true, true, true, style == "Dark chat");
            t.items[1].embed = Some(Embed { title: "Embed title".into(), desc: "Some **description** text".into(), color: "#e8523a".into(), thumb: String::new() });
            t.items[2].reply = Some(1);
            t.items[0].pinned = true;
            t.dividers = true;
            t.add(Item { kind: ItemKind::Divider, text: "Today".into(), ..Default::default() }, Some(0));
        }
        t.anim.on = false;
        all.push((style.replace(' ', "-"), t));
    }
    for (name, mode) in [("chat_typing", "dm"), ("chat_typing", "group"), ("chat_typing", "server"), ("comments_typing", "")] {
        let mut t = Thread::recipe(name, mode, "").unwrap();
        // the end of the typing: everything shown, then a second copy mid-typing
        t.anim.hold = 0.0;
        all.push((format!("{name}-{mode}-end"), t.clone()));
        all.push((format!("{name}-{mode}-off"), { let mut u = t.clone(); u.anim.on = false; u }));
    }
    for (name, t) in all {
        let (_, page) = html::prepare(&t.html(), &[]);
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.html")), page).unwrap();
    }
}
