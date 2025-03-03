#[macro_export]
macro_rules! select_next {
    ($vec:expr, $state:expr, $save_data_vec:expr) => {
        if let Some(idx) = $state.selected() {
            if $vec[idx].selected == Selected::Moving {
                if idx + 1 == $vec.len() {
                    $state.select_first();
                    $save_data_vec.swap(idx, 0);
                    $vec.swap(idx, 0);
                } else {
                    $state.select(Some(idx + 1));
                    $save_data_vec.swap(idx, idx + 1);
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
macro_rules! select_next_song {
    ($vec:expr, $state:expr, $save_data_vec:expr, $pl_vec:expr) => {
        if let Some(idx) = $state.selected() {
            if $vec[idx].selected == Selected::Moving {
                if idx + 1 == $vec.len() {
                    $state.select_first();
                    $save_data_vec.swap(idx, 0);
                    $vec.swap(idx, 0);
                    $pl_vec.swap(idx, 0);
                } else {
                    $state.select(Some(idx + 1));
                    $save_data_vec.swap(idx, idx + 1);
                    $vec.swap(idx, idx + 1);
                    $pl_vec.swap(idx, idx + 1);
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
    ($vec:expr, $state:expr, $save_data_vec:expr) => {
        if let Some(idx) = $state.selected() {
            if $vec[idx].selected == Selected::Moving {
                if idx == 0 {
                    let new_index = $vec.len() - 1;
                    $save_data_vec.swap(idx, new_index);
                    $state.select(Some(new_index));
                    $vec.swap(idx, new_index);
                } else {
                    $save_data_vec.swap(idx, idx - 1);
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
macro_rules! select_previous_song {
    ($vec:expr, $state:expr, $save_data_vec:expr, $pl_vec:expr) => {
        if let Some(idx) = $state.selected() {
            if $vec[idx].selected == Selected::Moving {
                if idx == 0 {
                    let new_index = $vec.len() - 1;
                    $save_data_vec.swap(idx, new_index);
                    $state.select(Some(new_index));
                    $vec.swap(idx, new_index);
                    $pl_vec.swap(idx, new_index);
                } else {
                    $save_data_vec.swap(idx, idx - 1);
                    $state.select(Some(idx - 1));
                    $vec.swap(idx, idx - 1);
                    $pl_vec.swap(idx, idx - 1);
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

#[macro_export]
macro_rules! moving_warning {
    ($item:expr, $log:expr) => {
        if $item.selected == Selected::Moving {
            $log = String::from("Can't change windows while moving an item");
            return;
        }
    };
}
