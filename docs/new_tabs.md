Phase one is complete... ignore 1.  we will move onto Problem 2. 

Well we aspire to add some new tabs in our boilerplate rust app. However when we learned our historic usage of shadows in the application causes a lot of repaint/redraw issues and UI responsiveness suffers as a consequence.

/opt/templates/rust/config.json

## Problem 1.

We aspire to add an Options tab in our application, this loads all of the configuration options from the config.json file allowing us to change them in the app itself. We aspire to add a prefix to all of the items in config.json which indicates what type of control it is, for example

btn_ = button
input_ = text box/inputbox
switch_ = bootstrap switch
check_ = traditional checkbox
radio_ = bootstrap radio checkbox
range_ = range slider
list_ = list group
select_ selection combo box

The difficult part is consigning what value each control has in config.json, for example;

check_start_minimized:1
check_rounded_corners:1
text_my_text_box:"Random nonsensical text"

Here we basically use a colon : as a suffix which denotes the value.  This gets problematic though when we want radio buttons or select boxes which have multiple values. Furthermore, some controls will be bigger vertically or horizontally so we will need a way to define width and height for some controls such as input/textboxes, further more some controls maybe read only and cannot be changed.


## Problem 2. Modals and offcanvas panels

Modals and offcanvas panels present a new problem, because we will most likely want to reintroduce shadows and effects so they look presentable. However historically when we had shadows and other effects in the application it made UI responsiveness slow when the window was maximized.  We would aspire to see if we can add some very resource friendly shadow effects.

We would want centered modals, left and right sliding modals etc.  If shadows or other effects even hamper the UI responisveness at all then I'd rather not add them in. It would be prudent to work cautiously on this.
