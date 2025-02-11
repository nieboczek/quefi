#[macro_export]
macro_rules! select_next {
    ($vec:expr, $state:expr) => {
        if let Some(idx) = $state.selected() {
            if $vec[idx].selected == Selected::Moving {
                if idx + 1 == $vec.len() {
                    $state.select_first();
                    $vec.swap(idx, 0);
                } else {
                    $state.select(Some(idx + 1));
                    $vec.swap(idx, idx + 1);
                }
                return;
            }
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
            if $vec[idx].selected == Selected::Moving {
                if idx == 0 {
                    let new_index = $vec.len() - 1;
                    $state.select(Some(new_index));
                    $vec.swap(idx, new_index);
                } else {
                    $state.select(Some(idx - 1));
                    $vec.swap(idx, idx - 1);
                }
                return;
            }
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

#[macro_export]
macro_rules! select {
    ($vec:expr, $state:expr, $idx:expr) => {
        $state.select(Some($idx));
        $vec[$idx].selected = Selected::Focused;
    };
}
