# Server info input

The vanilla edit component writes to a separate label. That label's
`#text_box_content` binding is `once`, renamed to `#item_name`, with
`property_bag.#property_field` marking the editable target. Host text must refresh
that label as well as the edit component. Ordinary once-bound labels stay seeded.
The component also supplies the focused border, placeholder target and text target;
launcher feedback uses those targets for the caret and selection.

References:


`menu::server_input_tests::real_carrier_server_fields_click_type_select_paste_save_and_play`
loads the installed pinned carrier and language table, uses the real Bevy input
system, and publishes every input frame at DPI 1 and Retina DPI 2. Both Add and Edit
exercise all three
fields through pointer focus, typing, deletion, select-all, paste, Tab, reverse
Tab and Escape. Assertions inspect the exact field's rendered text, caret and
selection and reject changes in the other fields. Save reopens persisted values;
Play queues the persisted host and port without opening a network connection.
PNG output is opt-in through the existing snapshot directory variable.

The pre-fix sequence failed on the first typed Name character: host state changed
but the child label stayed empty. `e3736c98` addressed only the edit component,
leaving this authored label path stale. This is the remaining omission in that
partial fix, not evidence that it introduced the original input defect.
