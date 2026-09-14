use super::*;
impl App {
    pub(super) fn aimed_book(&self) -> Option<crate::lore_books::Book> {
        let eye = self.camera.eye_position();
        let dir = self.camera.forward();
        crate::lore_books::nearby(&self.world, self.player.position)
            .into_iter()
            .filter(|b| crate::lore_books::can_read(&self.world, *b, self.player.position))
            .filter(|b| {
                let delta = b.pos + Vec3::Y * crate::lore_books::FOCUS_HEIGHT - eye;
                let t = dir.dot(delta);
                t > 0. && (delta - dir * t).length_squared() < 0.25
            })
            .max_by(|a, b| {
                dir.dot(
                    (a.pos + Vec3::Y * crate::lore_books::FOCUS_HEIGHT - eye).normalize_or_zero(),
                )
                .total_cmp(&dir.dot(
                    (b.pos + Vec3::Y * crate::lore_books::FOCUS_HEIGHT - eye).normalize_or_zero(),
                ))
            })
    }
    pub(super) fn open_recipe_book(&mut self, kind: u8) {
        if kind >= 4 {
            return;
        }
        self.audio.play_lore_book();
        self.crafting_ui.open = true;
        self.crafting_ui.show_book(kind);
        self.sync_settings_input();
    }
    pub(super) fn read_recipe_book(&mut self, from: Option<Peer>, sector: (i32, i32, u8)) {
        if !self.adventure_actor_alive(from) {
            return;
        }
        let (pos, key) = if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else {
                return;
            };
            let Some(rp) = host
                .clients
                .get(&peer)
                .and_then(|id| host.remote_players.get(id))
            else {
                return;
            };
            (rp.pos, Some(peer.account_key(&rp.nickname)))
        } else {
            (self.player.position, None)
        };
        crate::crafting::load_interaction_area(&mut self.world, pos);
        let account = if let Some(key) = key {
            self.guest_accounts.entry(key).or_default()
        } else {
            &mut self.player.crafting
        };
        let result = crate::lore_books::read(&self.world, account, pos, sector);
        if let Some(peer) = from {
            if let NetRole::Host(host) = &mut self.net {
                host.reliable.send(
                    &host.socket,
                    peer,
                    ReliableMsg::CraftState {
                        account: account.clone(),
                        feedback: None,
                    },
                );
                host.reliable
                    .send(&host.socket, peer, ReliableMsg::RecipeBookResult(result));
            }
        } else {
            match result {
                Ok(kind) => self.open_recipe_book(kind),
                Err(e) => self.toasts.push(Toast::new(e)),
            }
        }
    }
}
