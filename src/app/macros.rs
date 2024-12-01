#[macro_export]
macro_rules! select_next {
    ($vec:expr, $state:expr) => {
        if let Some(idx) = $state.selected() {
            if idx + 1 == $vec.len() {
                $vec[idx].selected = Selected::None;
                $state.select_first();
                $vec[0].selected = Selected::Focused;
            } else {
                $vec[idx].selected = Selected::None;
                $state.select(Some(idx + 1));
                $vec[idx + 1].selected = Selected::Focused;
            }
        }
    };
}

#[macro_export]
macro_rules! select_previous {
    ($vec:expr, $state:expr) => {
        if let Some(idx) = $state.selected() {
            if idx == 0 {
                $vec[idx].selected = Selected::None;
                let new_index = $vec.len() - 1;
                $state.select(Some(new_index));
                $vec[new_index].selected = Selected::Focused;
            } else {
                $vec[idx].selected = Selected::None;
                $state.select(Some(idx - 1));
                $vec[idx - 1].selected = Selected::Focused;
            }
        }
    };
}
