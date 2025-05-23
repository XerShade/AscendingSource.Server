use chrono::Duration;
use mio::Token;

use crate::{containers::*, gametypes::*, socket::*, sql::*, tasks::*};

pub fn player_switch_maps(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    new_pos: Position,
) -> Result<(Position, bool)> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        if let Some(mapref) = storage.maps.get(&p_data.movement.pos.map) {
            let mut map = mapref.borrow_mut();
            map.remove_player(storage, entity);
            map.remove_entity_from_grid(p_data.movement.pos);
        } else {
            return Ok((p_data.movement.pos, false));
        }

        if let Some(mapref) = storage.maps.get(&new_pos.map) {
            let mut map = mapref.borrow_mut();
            map.add_player(storage, entity);
            map.add_entity_to_grid(new_pos);
        } else {
            return Ok((p_data.movement.pos, false));
        }

        p_data.movement.pos = new_pos;

        Ok((p_data.movement.pos, true))
    } else {
        Err(AscendingError::missing_entity())
    }
}

pub fn player_swap_pos(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    pos: Position,
) -> Result<Position> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        let old_position = p_data.movement.pos;

        if p_data.movement.pos != pos {
            p_data.movement.pos = pos;

            let mut map = match storage.maps.get(&old_position.map) {
                Some(map) => map,
                None => return Ok(old_position),
            }
            .borrow_mut();

            map.remove_entity_from_grid(old_position);
            map.add_entity_to_grid(pos);
        }

        Ok(p_data.movement.pos)
    } else {
        Ok(Position::default())
    }
}

pub fn player_add_up_vital(world: &mut World, entity: GlobalKey, vital: usize) -> Result<i32> {
    Ok(
        if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
            let p_data = p_data.try_lock()?;

            let hp = p_data.combat.vitals.vitalmax[vital]
                .saturating_add(p_data.combat.vitals.vitalbuffs[vital]);

            if hp.is_negative() || hp == 0 { 1 } else { hp }
        } else {
            1
        },
    )
}

pub fn player_set_vital(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    vital: VitalTypes,
    amount: i32,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        p_data.combat.vitals.vital[vital as usize] =
            amount.min(p_data.combat.vitals.vitalmax[vital as usize]);

        DataTaskToken::Vitals(p_data.movement.pos.map).add_task(
            storage,
            vitals_packet(
                entity,
                p_data.combat.vitals.vital,
                p_data.combat.vitals.vitalmax,
            )?,
        )?;
    }

    Ok(())
}

#[inline]
pub fn player_give_vals(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    amount: u64,
) -> Result<u64> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let (player_money, socket_id) = {
            let p_data = p_data.try_lock()?;
            (p_data.money, p_data.socket.id)
        };
        let rem = u64::MAX.saturating_sub(player_money.vals);

        if rem > 0 {
            let mut cur = amount;
            if rem >= cur {
                let mut p_data = p_data.try_lock()?;
                p_data.money.vals = p_data.money.vals.saturating_add(cur);

                cur = 0;
            } else {
                p_data.try_lock()?.money.vals = u64::MAX;

                cur = cur.saturating_sub(rem);
            }

            send_money(world, storage, entity)?;
            update_currency(storage, world, entity)?;
            send_fltalert(
                storage,
                socket_id,
                format!("You Have Received {} Vals.", amount - cur),
                FtlType::Money,
            )?;
            return Ok(cur);
        }
    }

    Ok(amount)
}

#[inline]
pub fn player_take_vals(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    amount: u64,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut cur = amount;

        let socket_id = {
            let mut p_data = p_data.try_lock()?;

            if p_data.money.vals >= cur {
                p_data.money.vals = p_data.money.vals.saturating_sub(cur);
            } else {
                cur = p_data.money.vals;

                p_data.money.vals = 0;
            }

            p_data.socket.id
        };

        send_money(world, storage, entity)?;
        update_currency(storage, world, entity)?;
        send_fltalert(
            storage,
            socket_id,
            format!("You Lost {} Vals.", cur),
            FtlType::Money,
        )?;
    }
    Ok(())
}

pub fn send_login_info(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    code: String,
    handshake: String,
    socket_id: Token,
    username: String,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        p_data.relogin_code.code.insert(code.to_owned());
        p_data.login_handshake.handshake = handshake.to_owned();
    }

    storage.player_names.borrow_mut().insert(username, entity);
    storage
        .player_code
        .borrow_mut()
        .insert(code.to_owned(), entity);

    send_myindex(storage, socket_id, entity)?;
    send_codes(storage, code, handshake, socket_id)
}

pub fn send_reconnect_info(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    code: String,
    handshake: String,
    socket_id: Token,
    username: String,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        p_data.relogin_code.code.insert(code.to_owned());
        p_data.login_handshake.handshake = handshake.to_owned();
    }

    storage.player_names.borrow_mut().insert(username, entity);
    storage
        .player_code
        .borrow_mut()
        .insert(code.to_owned(), entity);

    send_myindex(storage, socket_id, entity)?;
    send_codes(storage, code, handshake, socket_id)
}

pub fn send_tls_reconnect(
    world: &mut World,
    storage: &Storage,
    entity: GlobalKey,
    code: String,
    handshake: String,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(entity) {
        let mut p_data = p_data.try_lock()?;

        p_data.relogin_code.code.insert(code.to_owned());
        p_data.login_handshake.handshake = handshake.to_owned();
    }

    storage
        .player_code
        .borrow_mut()
        .insert(code.to_owned(), entity);

    send_tls_codes(world, storage, entity, code, handshake)
}

pub fn reconnect_player(
    world: &mut World,
    storage: &Storage,
    old_entity: GlobalKey,
    new_socket: Socket,
) -> Result<()> {
    if let Some(Entity::Player(p_data)) = world.get_opt_entity(old_entity) {
        let tick = *storage.gettick.borrow();

        if let Some(client) = storage.server.borrow().clients.get(&new_socket.id) {
            client.borrow_mut().entity = Some(old_entity);
        }

        let position = {
            let mut p_data = p_data.try_lock()?;

            p_data.online_type = OnlineType::Accepted;

            p_data.movement.pos
        };

        let _ = storage.player_timeout.borrow_mut().insert(
            old_entity,
            PlayerConnectionTimer(tick + Duration::try_milliseconds(600000).unwrap_or_default()),
        );

        if let Some(map) = storage.maps.get(&position.map) {
            map.borrow_mut().remove_entity_from_grid(position);
        }
    }

    Ok(())
}
