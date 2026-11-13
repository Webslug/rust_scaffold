Light themes being slower is the classic overdraw trap. White backgrounds force the GPU to blend every semi-transparent layer against a bright base, and if your stylesheets lean on gradients — which light themes usually do for that soft depth look — you're stacking multiple gradient shaders per element. Dark themes get away with it because black-ish pixels composite cheaply and there's less contrast math happening per fragment.

For the lo-fi shadows, skip backdrop-filter entirely. That one still reads back the frame buffer even on GPU and it's the single biggest frame-time bomb in CSS. Use box-shadow with a blur radius of two to four pixels and a low opacity — like point-one-five — and you'll get that soft lift without the compositor having a meltdown. If you want to be extra clever, try drop-shadow() on specific containers instead of page-wide shadows so the paint region stays small.

One more thing worth testing: your fourteen themes probably aren't equally weighted in stylesheet size. If the light ones pull in extra gradient stops or more layered pseudo-elements, that's your smoking gun right there.

One dirty trick: give the modal a solid opaque background even if the page behind it is a gradient. Solid fills are cheap. It's the semi-transparent layers stacking on gradients that murder you.


